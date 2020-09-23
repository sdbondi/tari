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

use log::*;
use std::{cmp, fs, path::Path, sync::Arc, time::Duration};
use tari_app_utilities::{identity_management, utilities};
use tari_common::{CommsTransport, GlobalConfig, TorControlAuthentication};
use tari_comms::{
    peer_manager::Peer,
    socks,
    tor,
    tor::TorIdentity,
    transports::SocksConfig,
    utils::multiaddr::multiaddr_to_socketaddr,
    NodeIdentity,
};
use tari_comms_dht::{DbConnectionUrl, DhtConfig};
use tari_core::{
    base_node::{
        chain_metadata_service::ChainMetadataServiceInitializer,
        service::{BaseNodeServiceConfig, BaseNodeServiceInitializer},
        state_machine_service::initializer::BaseNodeStateMachineInitializer,
    },
    chain_storage::{BlockchainBackend, BlockchainDatabase},
    consensus::ConsensusManager,
    mempool::{Mempool, MempoolServiceConfig, MempoolServiceInitializer},
    transactions::types::CryptoFactories,
};
use tari_p2p::{
    comms_connector::pubsub_connector,
    initialization::{CommsConfig, P2pInitializer},
    services::liveness::{LivenessConfig, LivenessInitializer},
    transport::{TorConfig, TransportType},
};
use tari_service_framework::{handles::ServiceHandles, StackBuilder};
use tari_shutdown::ShutdownSignal;
use tokio::runtime;

const LOG_TARGET: &str = "c::bn::initialization";
/// The minimum buffer size for the base node pubsub_connector channel
const BASE_NODE_BUFFER_MIN_SIZE: usize = 30;
/// The minimum buffer size for the base node wallet pubsub_connector channel
const BASE_NODE_WALLET_BUFFER_MIN_SIZE: usize = 300;

pub struct BaseNodeBootstrapper<'a, B> {
    pub config: &'a GlobalConfig,
    pub node_identity: Arc<NodeIdentity>,
    pub db: BlockchainDatabase<B>,
    pub mempool: Mempool<B>,
    pub rules: ConsensusManager,
    pub factories: CryptoFactories,
    pub interrupt_signal: ShutdownSignal,
    pub seed_peers: Vec<Peer>,
}

impl<B> BaseNodeBootstrapper<'_, B>
where B: BlockchainBackend + 'static
{
    pub async fn bootstrap(self) -> Result<ServiceHandles, anyhow::Error> {
        let config = self.config;

        fs::create_dir_all(&config.peer_db_path)?;

        let buf_size = cmp::max(BASE_NODE_BUFFER_MIN_SIZE, config.buffer_size_base_node);
        let (publisher, subscription_factory) =
            pubsub_connector(runtime::Handle::current(), buf_size, config.buffer_rate_limit_base_node);
        let subscription_factory = Arc::new(subscription_factory);

        let node_config = BaseNodeServiceConfig::default(); // TODO - make this configurable
        let mempool_config = MempoolServiceConfig::default(); // TODO - make this configurable

        let comms_config = create_comms_config(config, self.node_identity);

        let sync_strategy = config.block_sync_strategy.parse().unwrap();

        let handles = StackBuilder::new(runtime::Handle::current(), self.interrupt_signal)
            .add_initializer(P2pInitializer::new(comms_config, publisher, self.seed_peers))
            .add_initializer(BaseNodeServiceInitializer::new(
                subscription_factory.clone(),
                self.db.clone(),
                self.mempool.clone(),
                self.rules.clone(),
                node_config,
            ))
            .add_initializer(MempoolServiceInitializer::new(
                subscription_factory.clone(),
                self.mempool,
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
            ))
            .add_initializer(ChainMetadataServiceInitializer)
            .add_initializer(BaseNodeStateMachineInitializer::new(
                self.db,
                self.rules,
                self.factories,
                sync_strategy,
            ))
            .finish()
            .await?;

        Ok(handles)
    }
}

fn create_comms_config(global: &GlobalConfig, node_identity: Arc<NodeIdentity>) -> CommsConfig {
    CommsConfig {
        node_identity,
        transport_type: setup_transport_type(global),
        datastore_path: global.peer_db_path.clone(),
        peer_database_name: "peers".to_string(),
        max_concurrent_inbound_tasks: 100,
        outbound_buffer_size: 100,
        // TODO - make this configurable
        dht: DhtConfig {
            database_url: DbConnectionUrl::File(global.data_dir.join("dht.db")),
            auto_join: true,
            ..Default::default()
        },
        // TODO: This should be false unless testing locally - make this configurable
        allow_test_addresses: true,
        listener_liveness_allowlist_cidrs: global.listener_liveness_allowlist_cidrs.clone(),
        listener_liveness_max_sessions: global.listnener_liveness_max_sessions,
        user_agent: format!("tari/basenode/{}", env!("CARGO_PKG_VERSION")),
    }
}

/// Creates a transport type from the given configuration
/// /// ## Paramters
/// `config` - The reference to the configuration in which to set up the comms stack, see [GlobalConfig]
///
/// ##Returns
/// TransportType based on the configuration
fn setup_transport_type(config: &GlobalConfig) -> TransportType {
    debug!(target: LOG_TARGET, "Transport is set to '{:?}'", config.comms_transport);

    match config.comms_transport.clone() {
        CommsTransport::Tcp {
            listener_address,
            tor_socks_address,
            tor_socks_auth,
        } => TransportType::Tcp {
            listener_address,
            tor_socks_config: tor_socks_address.map(|proxy_address| SocksConfig {
                proxy_address,
                authentication: tor_socks_auth
                    .map(utilities::into_socks_authentication)
                    .unwrap_or_default(),
            }),
        },
        CommsTransport::TorHiddenService {
            control_server_address,
            socks_address_override,
            forward_address,
            auth,
            onion_port,
        } => {
            let tor_identity_path = Path::new(&config.tor_identity_file);
            let identity = if tor_identity_path.exists() {
                // If this fails, we can just use another address
                identity_management::load_from_json::<_, TorIdentity>(&tor_identity_path).ok()
            } else {
                None
            };
            info!(
                target: LOG_TARGET,
                "Tor identity at path '{}' {:?}",
                tor_identity_path.to_string_lossy(),
                identity
                    .as_ref()
                    .map(|ident| format!("loaded for address '{}.onion'", ident.service_id))
                    .or_else(|| Some("not found".to_string()))
                    .unwrap()
            );

            let forward_addr = multiaddr_to_socketaddr(&forward_address).expect("Invalid tor forward address");
            TransportType::Tor(TorConfig {
                control_server_addr: control_server_address,
                control_server_auth: {
                    match auth {
                        TorControlAuthentication::None => tor::Authentication::None,
                        TorControlAuthentication::Password(password) => tor::Authentication::HashedPassword(password),
                    }
                },
                identity: identity.map(Box::new),
                port_mapping: (onion_port, forward_addr).into(),
                // TODO: make configurable
                socks_address_override,
                socks_auth: socks::Authentication::None,
            })
        },
        CommsTransport::Socks5 {
            proxy_address,
            listener_address,
            auth,
        } => TransportType::Socks {
            socks_config: SocksConfig {
                proxy_address,
                authentication: utilities::into_socks_authentication(auth),
            },
            listener_address,
        },
    }
}
