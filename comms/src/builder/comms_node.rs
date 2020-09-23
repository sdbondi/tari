// Copyright 2020, The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use super::{CommsBuilderError, CommsShutdown};
use crate::{
    backoff::BoxedBackoff,
    connection_manager::{ConnectionManager, ConnectionManagerEvent, ConnectionManagerRequester},
    connectivity::{ConnectivityEventRx, ConnectivityManager, ConnectivityRequester},
    multiaddr::Multiaddr,
    peer_manager::{NodeIdentity, Peer, PeerManager},
    protocol::{messaging, ProtocolExtensionContext, ProtocolExtensions},
    tor,
    transports::Transport,
};
use futures::{AsyncRead, AsyncWrite, StreamExt};
use log::*;
use std::{sync::Arc, time::Duration};
use tari_shutdown::{OptionalShutdownSignal, ShutdownSignal};
use tokio::{sync::broadcast, time};

#[cfg(feature = "rpc")]
use crate::protocol::ProtocolExtension;
use crate::{
    connection_manager::{ConnectionManagerConfig, ConnectionManagerRequest},
    connectivity::{ConnectivityConfig, ConnectivityRequest},
    noise::NoiseConfig,
    protocol::messaging::MessagingEventSender,
};
use futures::channel::mpsc;

const LOG_TARGET: &str = "comms::node";

/// Contains the built comms services
pub struct BuiltCommsNode<TTransport> {
    pub connection_manager_request_rx: mpsc::Receiver<ConnectionManagerRequest>,
    pub connection_manager_requester: ConnectionManagerRequester,
    pub connection_manager_config: ConnectionManagerConfig,
    pub connectivity_requester: ConnectivityRequester,
    pub connectivity_rx: mpsc::Receiver<ConnectivityRequest>,
    pub connectivity_config: ConnectivityConfig,
    pub dial_backoff: BoxedBackoff,
    pub node_identity: Arc<NodeIdentity>,
    pub hidden_service_ctl: Option<tor::HiddenServiceController>,
    pub peer_manager: Arc<PeerManager>,
    pub protocol_extensions: ProtocolExtensions,
    pub transport: TTransport,
    pub messaging_event_sender: Option<MessagingEventSender>,
    pub shutdown_signal: OptionalShutdownSignal,
}

impl<TTransport> BuiltCommsNode<TTransport>
where
    TTransport: Transport + Unpin + Send + Sync + Clone + 'static,
    TTransport::Output: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
{
    /// Add an RPC server/router in this instance of Tari comms.
    ///
    /// ```compile_fail
    /// # use tari_comms::CommsBuilder;
    /// # use tari_comms::protocol::rpc::RpcServer;
    /// let server = RpcServer::new().add_service(MyService).add_service(AnotherService);
    /// CommsBuilder::new().add_rpc_service(server).build();
    /// ```
    #[cfg(feature = "rpc")]
    pub fn add_rpc<T: ProtocolExtension + 'static>(self, rpc: T) -> Self {
        self.add_protocol_extension(rpc)
    }

    pub fn add_protocol_extension<T: ProtocolExtension + 'static>(mut self, extension: T) -> Self {
        self.protocol_extensions.add(extension);
        self
    }

    pub fn add_protocol_extensions(mut self, extensions: ProtocolExtensions) -> Self {
        self.protocol_extensions.extend(extensions);
        self
    }

    pub fn with_messaging_event_sender(mut self, messaging_event_sender: MessagingEventSender) -> Self {
        self.messaging_event_sender = Some(messaging_event_sender);
        self
    }

    pub async fn add_peers<I: IntoIterator<Item = Peer>>(self, peers: I) -> Result<Self, CommsBuilderError> {
        for peer in peers.into_iter() {
            self.peer_manager.add_peer(peer).await?;
        }
        Ok(self)
    }

    /// Wait until the ConnectionManager emits a Listening event. This is the signal that comms is ready.
    async fn wait_listening(
        mut events: broadcast::Receiver<Arc<ConnectionManagerEvent>>,
    ) -> Result<Multiaddr, CommsBuilderError> {
        loop {
            let event = time::timeout(Duration::from_secs(10), events.next())
                .await
                .map_err(|_| CommsBuilderError::ConnectionManagerEventStreamTimeout)?
                .ok_or(CommsBuilderError::ConnectionManagerEventStreamClosed)?
                .map_err(|_| CommsBuilderError::ConnectionManagerEventStreamLagged)?;

            match &*event {
                ConnectionManagerEvent::Listening(addr) => return Ok(addr.clone()),
                ConnectionManagerEvent::ListenFailed(err) => return Err(err.clone().into()),
                _ => {},
            }
        }
    }

    pub async fn spawn(self) -> Result<CommsNode, CommsBuilderError> {
        let BuiltCommsNode {
            connection_manager_requester,
            connection_manager_request_rx,
            connection_manager_config,
            connectivity_requester,
            connectivity_rx,
            connectivity_config,
            dial_backoff,
            node_identity,
            transport,
            peer_manager,
            protocol_extensions,
            messaging_event_sender,
            hidden_service_ctl,
            shutdown_signal,
        } = self;

        //---------------------------------- Connectivity Manager --------------------------------------------//
        let connectivity_manager = ConnectivityManager {
            config: connectivity_config,
            request_rx: connectivity_rx,
            event_tx: connectivity_requester.get_event_publisher(),
            connection_manager: connection_manager_requester.clone(),
            peer_manager: peer_manager.clone(),
            shutdown_signal: shutdown_signal.clone(),
        };

        let mut ext_context = ProtocolExtensionContext::new(
            connectivity_requester.clone(),
            peer_manager.clone(),
            shutdown_signal.clone(),
        );
        debug!(
            target: LOG_TARGET,
            "Installing {} protocol extension(s)",
            protocol_extensions.len()
        );
        protocol_extensions.install_all(&mut ext_context)?;

        //---------------------------------- Connection Manager --------------------------------------------//
        let noise_config = NoiseConfig::new(Arc::clone(&node_identity));

        let mut connection_manager = ConnectionManager::new(
            connection_manager_config,
            transport,
            noise_config,
            dial_backoff,
            connection_manager_request_rx,
            node_identity.clone(),
            peer_manager.clone(),
            connection_manager_requester.get_event_publisher(),
            shutdown_signal.clone(),
        );

        ext_context.register_complete_signal(connection_manager.complete_signal());
        connection_manager.set_protocols(ext_context.take_protocols().expect("Protocols already taken"));
        // Subscribe to events before spawning the actor to ensure that no events are missed
        let connection_manager_event_subscription = connection_manager_requester.get_event_subscription();

        //---------------------------------- Spawn Actors --------------------------------------------//
        connectivity_manager.create().spawn();
        connection_manager.spawn();

        info!(target: LOG_TARGET, "Hello from comms!");
        info!(
            target: LOG_TARGET,
            "Your node's public key is '{}'",
            node_identity.public_key()
        );
        info!(
            target: LOG_TARGET,
            "Your node's network ID is '{}'",
            node_identity.node_id()
        );
        info!(
            target: LOG_TARGET,
            "Your node's public address is '{}'",
            node_identity.public_address()
        );

        let listening_addr = Self::wait_listening(connection_manager_event_subscription).await?;
        let mut hidden_service = None;
        if let Some(mut ctl) = hidden_service_ctl {
            ctl.set_proxied_addr(listening_addr.clone());
            let hs = ctl.create_hidden_service().await?;
            node_identity.set_public_address(hs.get_onion_address());
            hidden_service = Some(hs);
        }

        Ok(CommsNode {
            shutdown_signal,
            connection_manager_requester,
            connectivity_requester,
            listening_addr,
            node_identity,
            peer_manager,
            messaging_event_tx: messaging_event_sender.unwrap_or_else(|| broadcast::channel(1).0),
            hidden_service,
            complete_signals: ext_context.drain_complete_signals(),
        })
    }

    /// Return a cloned atomic reference of the PeerManager
    pub fn peer_manager(&self) -> Arc<PeerManager> {
        Arc::clone(&self.peer_manager)
    }

    /// Return a cloned atomic reference of the NodeIdentity
    pub fn node_identity(&self) -> Arc<NodeIdentity> {
        Arc::clone(&self.node_identity)
    }

    /// Return an owned copy of a ConnectionManagerRequester. Used to initiate connections to peers.
    pub fn connection_manager_requester(&self) -> ConnectionManagerRequester {
        self.connection_manager_requester.clone()
    }

    /// Return an owned copy of a ConnectivityRequester. This is the async interface to the ConnectivityManager
    pub fn connectivity(&self) -> ConnectivityRequester {
        self.connectivity_requester.clone()
    }

    /// Returns an owned `OptionalShutdownSignal`
    pub fn shutdown_signal(&self) -> OptionalShutdownSignal {
        self.shutdown_signal.clone()
    }
}

/// CommsNode is a handle to a comms node.
///
/// It allows communication with the internals of tari_comms. Note that if this handle is dropped, tari_comms will shut
/// down.
#[derive(Clone)]
pub struct CommsNode {
    /// The `OptionalShutdownSignal` for this node. Use `wait_until_shutdown` to asynchronously block until the
    /// shutdown signal is triggered.
    shutdown_signal: OptionalShutdownSignal,
    /// Requester object for the ConnectionManager
    connection_manager_requester: ConnectionManagerRequester,
    /// Requester for the ConnectivityManager
    connectivity_requester: ConnectivityRequester,
    /// Node identity for this node
    node_identity: Arc<NodeIdentity>,
    /// Shared PeerManager instance
    peer_manager: Arc<PeerManager>,
    /// Tari messaging broadcast event channel. A `broadcast::Sender` is kept because it can create subscriptions as
    /// needed.
    messaging_event_tx: messaging::MessagingEventSender,
    /// The resolved Ip-Tcp listening address.
    listening_addr: Multiaddr,
    /// `Some` if the comms node is configured to run via a hidden service, otherwise `None`
    hidden_service: Option<tor::HiddenService>,
    /// The 'reciprocal' shutdown signals for each comms service
    complete_signals: Vec<ShutdownSignal>,
}

impl CommsNode {
    /// Get a subscription to `ConnectionManagerEvent`s
    pub fn subscribe_connection_manager_events(&self) -> broadcast::Receiver<Arc<ConnectionManagerEvent>> {
        self.connection_manager_requester.get_event_subscription()
    }

    /// Get a subscription to `ConnectivityEvent`s
    pub fn subscribe_connectivity_events(&self) -> ConnectivityEventRx {
        self.connectivity_requester.get_event_subscription()
    }

    /// Return a subscription to OMS events. This will emit events sent _after_ this subscription was created.
    pub fn subscribe_messaging_events(&self) -> messaging::MessagingEventReceiver {
        self.messaging_event_tx.subscribe()
    }

    /// Return a cloned atomic reference of the PeerManager
    pub fn peer_manager(&self) -> Arc<PeerManager> {
        Arc::clone(&self.peer_manager)
    }

    /// Return a cloned atomic reference of the NodeIdentity
    pub fn node_identity(&self) -> Arc<NodeIdentity> {
        Arc::clone(&self.node_identity)
    }

    /// Return a reference to the NodeIdentity
    pub fn node_identity_ref(&self) -> &NodeIdentity {
        &self.node_identity
    }

    /// Return the Ip/Tcp address that this node is listening on
    pub fn listening_address(&self) -> &Multiaddr {
        &self.listening_addr
    }

    /// Return the Ip/Tcp address that this node is listening on
    pub fn hidden_service(&self) -> Option<&tor::HiddenService> {
        self.hidden_service.as_ref()
    }

    /// Return an owned copy of a ConnectionManagerRequester. Used to initiate connections to peers.
    pub fn connection_manager(&self) -> ConnectionManagerRequester {
        self.connection_manager_requester.clone()
    }

    /// Return an owned copy of a ConnectivityRequester. This is the async interface to the ConnectivityManager
    pub fn connectivity(&self) -> ConnectivityRequester {
        self.connectivity_requester.clone()
    }

    /// Returns a new `OptionalShutdownSignal`
    pub fn shutdown_signal(&self) -> OptionalShutdownSignal {
        self.shutdown_signal.clone()
    }

    /// Wait for comms to shutdown once the shutdown signal is triggered and for comms services to shut down.
    /// The object is consumed to ensure that no handles/channels are kept after shutdown
    pub fn wait_until_shutdown(self) -> CommsShutdown {
        CommsShutdown::new(
            self.shutdown_signal
                .into_signal()
                .into_iter()
                .chain(self.complete_signals),
        )
    }
}
