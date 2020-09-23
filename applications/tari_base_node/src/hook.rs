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

use std::{cmp, future::Future, sync::Arc, time::Duration};
use tari_comms_dht::outbound::OutboundMessageRequester;
use tari_core::{
    base_node::{
        chain_metadata_service::ChainMetadataServiceInitializer,
        service::{BaseNodeServiceConfig, BaseNodeServiceInitializer},
        state_machine_service::initializer::BaseNodeStateMachineInitializer,
    },
    chain_storage::{BlockchainBackend, BlockchainDatabase},
    mempool::{Mempool, MempoolServiceConfig, MempoolServiceInitializer, MempoolSyncProtocolExtension},
};
use tari_p2p::{
    comms_connector,
    comms_connector::PubsubDomainConnector,
    hooks::{AsyncHook, BeforeSpawnContext, InitializationHookError},
    services::{
        comms_outbound::CommsOutboundServiceInitializer,
        liveness::{LivenessConfig, LivenessInitializer},
    },
};
use tari_service_framework::StackBuilder;
use tari_shutdown::ShutdownSignal;
use tokio::runtime;

const LOG_TARGET: &str = "c::bn::initialization";

pub struct BaseNodeInitializationHook<B> {
    mempool: Mempool<B>,
    db: BlockchainDatabase<B>,
    connector: PubsubDomainConnector,
    shutdown_signal: ShutdownSignal,
}

impl<B> BaseNodeInitializationHook<B> {
    pub fn new(mempool: Mempool<B>) -> Self {
        Self { mempool }
    }

    /// Asynchronously registers services of the base node
    ///
    /// ## Parameters
    /// `comms` - A reference to the comms node. This is the communications stack
    /// `db` - The interface to the blockchain database, for all transactions stored in a block
    /// `dht` - A reference to the peer discovery service
    /// `subscription_factory` - The publish-subscribe messaging system, wrapped in an atomic reference counter
    /// `mempool` - The mempool interface, for all transactions not yet included or recently included in a block
    /// `consensus_manager` - The consensus manager for the blockchain
    /// `factories` -  Cryptographic factory based on Pederson Commitments
    ///
    /// ## Returns
    /// A hashmap of handles wrapped in an atomic reference counter
    async fn bootstrap_services(&self, outbound_requester: OutboundMessageRequester) {}
}

impl<B> AsyncHook<BeforeSpawnContext> for BaseNodeInitializationHook<B>
where B: BlockchainBackend + 'static
{
    type Future = impl Future<Output = Result<BeforeSpawnContext, InitializationHookError>>;

    fn call(&mut self, mut context: BeforeSpawnContext) -> Self::Future {
        // let buf_size = std::cmp::max(BASE_NODE_BUFFER_MIN_SIZE, config.buffer_size_base_node);
        // let (publisher, base_node_subscriptions) =
        //     pubsub_connector(handle.clone(), buf_size, config.buffer_rate_limit_base_node);
        // let base_node_subscriptions = Arc::new(base_node_subscriptions);
        // create_peer_db_folder(&config.peer_db_path)?;

        context.add_protocol_extension(MempoolSyncProtocolExtension::new(
            Default::default(),
            self.mempool.clone(),
        ));

        let shutdown_signal = self.shutdown_signal.clone();
        let connector = self.connector.clone();

        async move {
            // let buf_size = cmp::max(BASE_NODE_BUFFER_MIN_SIZE, config.buffer_size_base_node);
            // let (publisher, base_node_subscriptions) =
            //     comms_connector::pubsub_connector(handle.clone(), buf_size, config.buffer_rate_limit_base_node);
            // let base_node_subscriptions = Arc::new(base_node_subscriptions);
            // create_peer_db_folder(&config.peer_db_path)?;

            // async fn register_base_node_services<B>(
            //     comms: &CommsNode,
            //     dht: &Dht,
            //     db: BlockchainDatabase<B>,
            //     subscription_factory: Arc<SubscriptionFactory>,
            //     mempool: Mempool<B>,
            //     consensus_manager: ConsensusManager,
            //     factories: CryptoFactories,
            //     sync_strategy: BlockSyncStrategy,
            //     interrupt_signal: ShutdownSignal,
            // ) -> Arc<ServiceHandles>
            //     where
            //         B: BlockchainBackend + 'static,
            let node_config = BaseNodeServiceConfig::default(); // TODO - make this configurable
            let mempool_config = MempoolServiceConfig::default(); // TODO - make this configurable
            StackBuilder::new(runtime::Handle::current(), shutdown_signal)
                .add_initializer(CommsOutboundServiceInitializer::new(
                    context.outbound_requester().clone(),
                ))
                .add_initializer(BaseNodeServiceInitializer::new(
                    base_node_subscriptions.clone(),
                    self.db.clone(),
                    self.mempool.clone(),
                    self.consensus_manager.clone(),
                    node_config,
                ))
                .add_initializer(MempoolServiceInitializer::new(
                    subscription_factory.clone(),
                    self.mempool.clone(),
                    mempool_config,
                ))
                .add_initializer(LivenessInitializer::new(
                    LivenessConfig {
                        auto_ping_interval: Some(Duration::from_secs(30)),
                        refresh_neighbours_interval: Duration::from_secs(3 * 60),
                        random_peer_selection_ratio: 0.4,
                        ..Default::default()
                    },
                    subscription_factory,
                    dht.dht_requester(),
                ))
                .add_initializer(ChainMetadataServiceInitializer)
                .add_initializer(BaseNodeStateMachineInitializer::new(
                    db.clone(),
                    consensus_manager.clone(),
                    factories.clone(),
                    sync_strategy,
                    comms.peer_manager(),
                    comms.connectivity(),
                    interrupt_signal,
                ))
                .finish()
                .await
                .expect("Service initialization failed");

            println!("WAITING");
            tokio::time::delay_for(std::time::Duration::from_secs(5)).await;

            // debug!(target: LOG_TARGET, "Registering base node services");
            // let base_node_handles = register_base_node_services(
            //     &base_node_comms,
            //     &base_node_dht,
            //     db.clone(),
            //     base_node_subscriptions.clone(),
            //     mempool,
            //     rules.clone(),
            //     factories.clone(),
            //     config
            //         .block_sync_strategy
            //         .parse()
            //         .expect("Problem reading block sync strategy from config"),
            //     interrupt_signal.clone(),
            // )
            //     .await;
            // debug!(target: LOG_TARGET, "Base node service registration complete.");
            Ok(context)
        }
    }
}
