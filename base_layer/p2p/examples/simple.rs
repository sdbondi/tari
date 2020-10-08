//  Copyright 2020, The Tari Project
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

use futures::{future, StreamExt};
use rand::rngs::OsRng;
use std::{error::Error, sync::Arc};
use tari_comms::{NodeIdentity, UnspawnedCommsNode};
use tari_comms_dht::{domain_message::OutboundDomainMessage, Dht};
use tari_p2p::{
    comms_connector::{pubsub_connector, SubscriptionFactory},
    initialization,
    initialization::{CommsConfig, P2pInitializer},
    tari_message::TariMessageType,
    transport::TransportType,
};
use tari_service_framework::{ServiceInitializationError, ServiceInitializer, ServiceInitializerContext, StackBuilder};
use tari_shutdown::Shutdown;
use tokio::runtime;

#[tokio_macros::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let node1 = NodeIdentity::random(
        &mut OsRng,
        "/ip4/127.0.0.1/tcp/8000".parse().unwrap(),
        Default::default(),
    )
    .map(Arc::new)
    .unwrap();
    let node2 = NodeIdentity::random(
        &mut OsRng,
        "/ip4/127.0.0.1/tcp/8001".parse().unwrap(),
        Default::default(),
    )
    .map(Arc::new)
    .unwrap();

    let shutdown = Shutdown::new();

    // Node 1
    let config = CommsConfig {
        datastore_path: Default::default(),
        peer_database_name: "simple_node1".to_string(),
        max_concurrent_inbound_tasks: 1,
        outbound_buffer_size: 100,
        dht: Default::default(),

        transport_type: TransportType::Tcp {
            listener_address: node1.public_address(),
            tor_socks_config: None,
        },
        node_identity: node1,
        allow_test_addresses: true,
        listener_liveness_max_sessions: 0,
        listener_liveness_allowlist_cidrs: vec![],
        user_agent: "".to_string(),
    };
    let (publisher, subscriptions) = pubsub_connector(runtime::Handle::current(), 100, 5);
    let mut handles1 = StackBuilder::new(shutdown.to_signal())
        .add_initializer(P2pInitializer::new(config, publisher, vec![node2.to_peer()]))
        .add_initializer(EchoService(subscriptions))
        .build()
        .await
        .unwrap();

    let comms1 = handles1.take_handle::<UnspawnedCommsNode>().unwrap();
    let comms1 = initialization::spawn_comms_using_transport(comms1, TransportType::Tcp {
        listener_address: node2.public_address(),
        tor_socks_config: None,
    })
    .await?;

    // Node2
    let config = CommsConfig {
        datastore_path: Default::default(),
        peer_database_name: "simple_node1".to_string(),
        max_concurrent_inbound_tasks: 1,
        outbound_buffer_size: 100,
        dht: Default::default(),

        transport_type: TransportType::Tcp {
            listener_address: node2.public_address(),
            tor_socks_config: None,
        },

        node_identity: node2.clone(),
        allow_test_addresses: true,
        listener_liveness_max_sessions: 0,
        listener_liveness_allowlist_cidrs: vec![],
        user_agent: "".to_string(),
    };
    let (publisher, subscriptions) = pubsub_connector(runtime::Handle::current(), 100, 5);
    let mut handles2 = StackBuilder::new(shutdown.to_signal())
        .add_initializer(P2pInitializer::new(config, publisher, vec![]))
        .add_initializer(EchoService(subscriptions))
        .build()
        .await
        .unwrap();

    let comms2 = handles2.take_handle::<UnspawnedCommsNode>().unwrap();
    let comms2 = initialization::spawn_comms_using_transport(comms2, TransportType::Tcp {
        listener_address: node2.public_address(),
        tor_socks_config: None,
    })
    .await?;

    let mut dht1 = handles1.expect_handle::<Dht>().outbound_requester();
    let dht2 = handles2.expect_handle::<Dht>().outbound_requester();

    dht1.send_direct_node_id(
        node2.node_id().clone(),
        OutboundDomainMessage::new(TariMessageType::PingPong, "Jude".to_string()),
    )
    .await
    .unwrap();
    Ok(())
}

pub struct EchoService(SubscriptionFactory);

impl ServiceInitializer for EchoService {
    type Future = future::Ready<Result<(), ServiceInitializationError>>;

    fn initialize(&mut self, context: ServiceInitializerContext) -> Self::Future {
        let mut subscription = self.0.get_subscription(TariMessageType::PingPong, "123");
        context.spawn_when_ready(move |handles| async move {
            let dht = handles.expect_handle::<Dht>();
            let mut outbound = dht.outbound_requester();

            while let Some(msg) = subscription.next().await {
                outbound
                    .send_direct_node_id(
                        msg.source_peer.node_id.clone(),
                        OutboundDomainMessage::new(2, format!("Hey {}", String::from_utf8_lossy(&msg.body))),
                    )
                    .await
                    .unwrap();
            }
        });

        future::ready(Ok(()))
    }
}
