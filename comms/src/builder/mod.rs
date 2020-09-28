//  Copyright 2019 The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! # CommsBuilder
//!
//! The [CommsBuilder] provides a simple builder API for getting Tari comms p2p messaging up and running.
//!
//! [CommsBuilder]: ./builder/struct.CommsBuilder.html

mod comms_node;
pub use comms_node::{BuiltCommsNode, CommsNode};

mod shutdown;
pub use shutdown::CommsShutdown;

mod error;
pub use error::CommsBuilderError;

mod consts;
mod placeholder;

#[cfg(test)]
mod tests;

#[cfg(feature = "rpc")]
use crate::protocol::ProtocolExtension;

use crate::{
    backoff::{Backoff, BoxedBackoff, ExponentialBackoff},
    connection_manager::{ConnectionManagerConfig, ConnectionManagerRequester},
    connectivity::{ConnectivityConfig, ConnectivityRequester},
    multiaddr::Multiaddr,
    multiplexing::Substream,
    peer_manager::{NodeIdentity, PeerManager},
    protocol::{ProtocolExtensions, Protocols},
    tor,
    types::CommsDatabase,
};
use futures::channel::mpsc;
use log::*;
use std::sync::Arc;
use tari_shutdown::{OptionalShutdownSignal, ShutdownSignal};
use tokio::sync::broadcast;

const LOG_TARGET: &str = "comms::builder";

/// The `CommsBuilder` provides a simple builder API for getting Tari comms p2p messaging up and running.
pub struct CommsBuilder {
    peer_storage: Option<CommsDatabase>,
    node_identity: Option<Arc<NodeIdentity>>,
    dial_backoff: BoxedBackoff,
    hidden_service_ctl: Option<tor::HiddenServiceController>,
    connection_manager_config: ConnectionManagerConfig,
    connectivity_config: ConnectivityConfig,
    protocol_extensions: ProtocolExtensions,

    shutdown_signal: OptionalShutdownSignal,
}

impl Default for CommsBuilder {
    fn default() -> Self {
        Self {
            peer_storage: None,
            node_identity: None,
            dial_backoff: Box::new(ExponentialBackoff::default()),
            hidden_service_ctl: None,
            connection_manager_config: ConnectionManagerConfig::default(),
            connectivity_config: ConnectivityConfig::default(),
            protocol_extensions: ProtocolExtensions::new(),
            shutdown_signal: OptionalShutdownSignal::none(),
        }
    }
}

impl CommsBuilder {
    /// Set the [NodeIdentity] for this comms instance. This is required.
    ///
    /// [OutboundMessagePool]: ../../outbound_message_service/index.html#outbound-message-pool
    pub fn with_node_identity(mut self, node_identity: Arc<NodeIdentity>) -> Self {
        self.node_identity = Some(node_identity);
        self
    }

    /// Set the shutdown signal for this comms instance
    pub fn with_shutdown_signal(mut self, shutdown_signal: ShutdownSignal) -> Self {
        self.shutdown_signal.set(shutdown_signal);
        self
    }

    /// Set the user agent string for this comms node. This string is sent once when establishing a connection.
    pub fn with_user_agent<T: ToString>(mut self, user_agent: T) -> Self {
        self.connection_manager_config.user_agent = user_agent.to_string();
        self
    }

    /// Allow test addresses (memory addresses, local loopback etc). This should only be activated for tests.
    pub fn allow_test_addresses(mut self) -> Self {
        #[cfg(not(debug_assertions))]
        warn!(
            target: LOG_TARGET,
            "Test addresses are enabled! This is invalid and potentially insecure when running a production node."
        );
        self.connection_manager_config.allow_test_addresses = true;
        self
    }

    pub fn with_listener_address(mut self, listener_address: Multiaddr) -> Self {
        self.connection_manager_config.listener_address = listener_address;
        self
    }

    pub fn with_listener_liveness_max_sessions(mut self, max_sessions: usize) -> Self {
        self.connection_manager_config.liveness_max_sessions = max_sessions;
        self
    }

    pub fn with_listener_liveness_allowlist_cidrs(mut self, cidrs: Vec<cidr::AnyIpCidr>) -> Self {
        self.connection_manager_config.liveness_cidr_allowlist = cidrs;
        self
    }

    /// The maximum number of connection tasks that will be spawned at the same time. Once this limit is reached, peers
    /// attempting to connect will have to wait for another connection attempt to complete.
    pub fn with_max_simultaneous_inbound_connects(mut self, max_simultaneous_inbound_connects: usize) -> Self {
        self.connection_manager_config.max_simultaneous_inbound_connects = max_simultaneous_inbound_connects;
        self
    }

    /// The number of dial attempts to make before giving up.
    pub fn with_max_dial_attempts(mut self, max_dial_attempts: usize) -> Self {
        self.connection_manager_config.max_dial_attempts = max_dial_attempts;
        self
    }

    /// Sets the minimum required connectivity as a percentage of peers added to the connectivity manager peer set.
    pub fn with_min_connectivity(mut self, min_connectivity: f32) -> Self {
        self.connectivity_config.min_connectivity = min_connectivity;
        self
    }

    /// Call to disable connection reaping. Usually you would want to have this enabled, however there are some test
    /// cases where disabling this is desirable.
    pub fn disable_connection_reaping(mut self) -> Self {
        self.connectivity_config.is_connection_reaping_enabled = false;
        self
    }

    /// Set the peer storage database to use.
    pub fn with_peer_storage(mut self, peer_storage: CommsDatabase) -> Self {
        self.peer_storage = Some(peer_storage);
        self
    }

    // /// Configure the `CommsBuilder` to build a node which communicates using the given `tor::HiddenService`.
    // pub async fn configure_from_hidden_service(
    //     mut self,
    //     mut hidden_service_ctl: tor::HiddenServiceController,
    // ) -> Result<CommsBuilder<SocksTransport>, CommsBuilderError>
    // {
    //     // Set the listener address to be the address (usually local) to which tor will forward all traffic
    //     self.connection_manager_config.listener_address = hidden_service_ctl.proxied_address();
    //     let transport = hidden_service_ctl.get_transport().await?;
    //
    //     Ok(CommsBuilder {
    //         // Set the socks transport configured for this hidden service
    //         transport,
    //         // Set the hidden service.
    //         hidden_service_ctl: Some(hidden_service_ctl),
    //         peer_storage: self.peer_storage,
    //         node_identity: self.node_identity,
    //         protocols: self.protocols,
    //         dial_backoff: self.dial_backoff,
    //         connection_manager_config: self.connection_manager_config,
    //         connectivity_config: self.connectivity_config,
    //         protocol_extensions: self.protocol_extensions,
    //         shutdown_signal: self.shutdown_signal,
    //     })
    // }

    /// Set the backoff that [ConnectionManager] uses when dialing peers. This is optional. If omitted the default
    /// ExponentialBackoff is used. [ConnectionManager]: crate::connection_manager::next::ConnectionManager
    pub fn with_dial_backoff<T>(mut self, backoff: T) -> Self
    where T: Backoff + Send + Sync + 'static {
        self.dial_backoff = Box::new(backoff);
        self
    }

    /// Add an RPC server/router in this instance of Tari comms.
    ///
    /// ```compile_fail
    /// # use tari_comms::CommsBuilder;
    /// # use tari_comms::protocol::rpc::RpcServer;
    /// let server = RpcServer::new().add_service(MyService).add_service(AnotherService);
    /// CommsBuilder::new().add_rpc_service(server).build();
    /// ```
    #[cfg(feature = "rpc")]
    pub fn add_rpc<T: ProtocolExtension + 'static>(mut self, rpc: T) -> Self {
        // Rpc router is treated the same as any other `ProtocolExtension` however this method may make it clearer for
        // users that this is the correct way to add the RPC server
        self.protocol_extensions.add(rpc);
        self
    }

    pub fn add_protocol_extensions(mut self, extensions: ProtocolExtensions) -> Self {
        self.protocol_extensions.extend(extensions);
        self
    }

    fn make_peer_manager(&mut self) -> Result<Arc<PeerManager>, CommsBuilderError> {
        match self.peer_storage.take() {
            Some(storage) => {
                // TODO: Peer manager should be refactored to be backend agnostic
                #[cfg(not(test))]
                PeerManager::migrate_lmdb(&storage.inner())?;

                let peer_manager = PeerManager::new(storage).map_err(CommsBuilderError::PeerManagerError)?;
                Ok(Arc::new(peer_manager))
            },
            None => Err(CommsBuilderError::PeerStorageNotProvided),
        }
    }

    /// Build the required comms services. Services will not be started.
    pub fn build(mut self) -> Result<BuiltCommsNode, CommsBuilderError> {
        debug!(target: LOG_TARGET, "Building comms");
        let node_identity = self
            .node_identity
            .take()
            .ok_or_else(|| CommsBuilderError::NodeIdentityNotSet)?;

        let peer_manager = self.make_peer_manager()?;

        //---------------------------------- Connection Manager --------------------------------------------//
        let (conn_man_tx, connection_manager_request_rx) =
            mpsc::channel(consts::CONNECTION_MANAGER_REQUEST_BUFFER_SIZE);
        let (connection_manager_event_tx, _) = broadcast::channel(consts::CONNECTION_MANAGER_EVENTS_BUFFER_SIZE);
        let connection_manager_requester = ConnectionManagerRequester::new(conn_man_tx, connection_manager_event_tx);

        //---------------------------------- ConnectivityManager --------------------------------------------//
        let (connectivity_tx, connectivity_rx) = mpsc::channel(consts::CONNECTIVITY_MANAGER_REQUEST_BUFFER_SIZE);
        let (event_tx, _) = broadcast::channel(consts::CONNECTIVITY_MANAGER_EVENTS_BUFFER_SIZE);
        let connectivity_requester = ConnectivityRequester::new(connectivity_tx, event_tx.clone());

        Ok(BuiltCommsNode {
            connection_manager_requester,
            connection_manager_request_rx,
            connection_manager_config: self.connection_manager_config,
            connectivity_requester,
            connectivity_rx,
            connectivity_config: self.connectivity_config,
            dial_backoff: self.dial_backoff,
            node_identity,
            peer_manager,
            protocol_extensions: self.protocol_extensions,
            hidden_service_ctl: self.hidden_service_ctl,
            messaging_event_sender: None,
            shutdown_signal: self.shutdown_signal,
        })
    }
}
