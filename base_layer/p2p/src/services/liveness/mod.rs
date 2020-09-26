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

//! # Liveness Service
//!
//! This service is responsible for sending pings to any peer as well as maintaining
//! some very basic counters for the number of ping/pongs sent and received.
//!
//! It is responsible for:
//! - handling requests to the Liveness backend. Types of requests can be found in the [LivenessRequest] enum, and
//! - reading incoming [PingPong] messages and processing them.
//!
//! In future, this service may be expanded to included periodic pings to maintain
//! latency and availability statistics for peers.
//!
//! [LivenessRequest]: ./messages/enum.LivenessRequets.html
//! [PingPong]: ./messages/enum.PingPong.html

mod config;
pub mod error;
mod handle;
mod message;
mod service;
mod state;

use self::{message::PingPongMessage, service::LivenessService, state::LivenessState};
use crate::{
    comms_connector::PeerMessage,
    domain_message::DomainMessage,
    services::utils::{map_decode, ok_or_skip_result},
    tari_message::TariMessageType,
};
use futures::{future, Future, Stream, StreamExt};
use log::*;
use std::sync::Arc;
use tari_comms_dht::{outbound::OutboundMessageRequester, Dht};
use tari_service_framework::{
    handles::ServiceHandlesFuture,
    reply_channel,
    ServiceInitializationError,
    ServiceInitializer,
};
use tari_shutdown::ShutdownSignal;
use tokio::runtime;

#[cfg(feature = "test-mocks")]
pub mod mock;

// Public exports
pub use self::{
    config::LivenessConfig,
    handle::{LivenessEvent, LivenessEventSender, LivenessHandle, LivenessRequest, LivenessResponse, PingPongEvent},
    state::Metadata,
};
use crate::comms_connector::TopicSubscriptionFactory;
pub use crate::proto::liveness::MetadataKey;
use tokio::sync::broadcast;

const LOG_TARGET: &str = "p2p::services::liveness";

/// Initializer for the Liveness service handle and service future.
pub struct LivenessInitializer {
    config: Option<LivenessConfig>,
    inbound_message_subscription_factory: Arc<TopicSubscriptionFactory<TariMessageType, Arc<PeerMessage>>>,
}

impl LivenessInitializer {
    /// Create a new LivenessInitializer from the inbound message subscriber
    pub fn new(
        config: LivenessConfig,
        inbound_message_subscription_factory: Arc<TopicSubscriptionFactory<TariMessageType, Arc<PeerMessage>>>,
    ) -> Self
    {
        Self {
            config: Some(config),
            inbound_message_subscription_factory,
        }
    }

    /// Get a stream of inbound PingPong messages
    fn ping_stream(&self) -> impl Stream<Item = DomainMessage<PingPongMessage>> {
        self.inbound_message_subscription_factory
            .get_subscription(TariMessageType::PingPong, "Liveness")
            .map(map_decode::<PingPongMessage>)
            .filter_map(ok_or_skip_result)
    }
}

impl ServiceInitializer for LivenessInitializer {
    type Future = impl Future<Output = Result<(), ServiceInitializationError>>;

    fn initialize(
        &mut self,
        _executor: runtime::Handle,
        handles: ServiceHandlesFuture,
        shutdown: ShutdownSignal,
    ) -> Self::Future
    {
        let (sender, receiver) = reply_channel::unbounded();

        let (publisher, _) = broadcast::channel(200);

        let liveness_handle = LivenessHandle::new(sender, publisher.clone());

        // Saving a clone
        let config = self
            .config
            .take()
            .expect("Liveness service initialized more than once.");

        // Register handle before waiting for handles to be ready
        handles.register(liveness_handle);

        // Create a stream which receives PingPong messages from comms
        let ping_stream = self.ping_stream();

        // Spawn the Liveness service on the executor
        handles.spawn_when_ready(|handles| async move {
            let dht = handles.expect_handle::<Dht>();
            let dht_requester = dht.dht_requester();

            let outbound_handle = handles.expect_handle::<OutboundMessageRequester>();

            let state = LivenessState::new();

            let service = LivenessService::new(
                config,
                receiver,
                ping_stream,
                state,
                dht_requester,
                outbound_handle,
                publisher,
                shutdown,
            );
            service.run().await;
            debug!(target: LOG_TARGET, "Liveness service has shut down");
        });

        future::ready(Ok(()))
    }
}
