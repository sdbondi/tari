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
use std::{cmp, fs, net::SocketAddr, sync::Arc, time::Duration};
use tari_app_utilities::{identity_management, utilities};
use tari_common::{CommsTransport, GlobalConfig, TorControlAuthentication};
use tari_comms::{
    multiaddr::{Multiaddr, Protocol},
    socks,
    tor,
    tor::TorIdentity,
    transports::SocksConfig,
    CommsNode,
    NodeIdentity,
    UnspawnedCommsNode,
};
use tari_comms_dht::{DbConnectionUrl, DhtConfig};
use tari_p2p::{
    comms_connector::pubsub_connector,
    hooks::P2pInitializationHooks,
    initialization,
    initialization::{CommsConfig, P2pInitializer},
    services::liveness::{LivenessConfig, LivenessInitializer},
    transport::{TorConfig, TransportType},
};
use tari_service_framework::{FinalServiceContext, StackBuilder};
use tari_shutdown::ShutdownSignal;
use tari_wallet::{
    output_manager_service::{
        config::OutputManagerServiceConfig,
        storage::sqlite_db::OutputManagerSqliteDatabase,
        OutputManagerServiceInitializer,
    },
    transaction_service::{config::TransactionServiceConfig, TransactionServiceInitializer},
};

const LOG_TARGET: &str = "c::bn::initialization:wallet";
/// The minimum buffer size for the base node wallet pubsub_connector channel
const BASE_NODE_WALLET_BUFFER_MIN_SIZE: usize = 300;

pub struct WalletBootstrapper<'a> {
    pub config: &'a GlobalConfig,
    pub node_identity: Arc<NodeIdentity>,
    pub interrupt_signal: ShutdownSignal,
}

impl WalletBootstrapper<'_> {
    pub async fn bootstrap(self) -> Result<(CommsNode, FinalServiceContext), anyhow::Error> {
        let buf_size = cmp::max(BASE_NODE_WALLET_BUFFER_MIN_SIZE, config.buffer_size_base_node_wallet);
        let (publisher, wallet_subscriptions) = pubsub_connector(buf_size, config.buffer_rate_limit_base_node_wallet);
        let subscription_factory = Arc::new(wallet_subscriptions);
        fs::create_dir_all(&self.config.wallet_peer_db_path)?;
        // let (wallet_comms, wallet_dht) = setup_wallet_comms(
        //     wallet_node_identity,
        //     config,
        //     publisher,
        //     base_node_comms.node_identity().to_peer(),
        //     interrupt_signal.clone(),
        // )
        //     .await?;
        // wallet_comms
        //     .connectivity()
        //     .add_managed_peers(vec![base_node_comms.node_identity().node_id().clone()])
        //     .await?;

        let mut hooks = P2pInitializationHooks::new();
        self.add_seed_peers(&mut hooks);

        identity_management::save_as_json(&config.wallet_identity_file, &*comms.node_identity())
            .map_err(|e| anyhow!("Failed to save node identity: {:?}", e))?;
        if let Some(hs) = comms.hidden_service() {
            identity_management::save_as_json(&config.wallet_tor_identity_file, hs.tor_identity())
                .map_err(|e| anyhow!("Failed to save tor identity: {:?}", e))?;
        }

        let mut handles = StackBuilder::new( self.interrupt_signal)
            .add_initializer(P2pInitializer::new(comms_config, publisher, hooks))
            .add_initializer(LivenessInitializer::new(
                LivenessConfig{
                    auto_ping_interval: Some(Duration::from_secs(60)),
                    ..Default::default()
                },
                subscription_factory.clone(),
            ))
            // Wallet services
            .add_initializer(OutputManagerServiceInitializer::new(
                OutputManagerServiceConfig {
                    base_node_query_timeout,
                    ..Default::default()
                },
                subscription_factory.clone(),
                OutputManagerSqliteDatabase::new(wallet_db_conn.clone(),None),
                factories.clone(),
                network
            ))
            .add_initializer(TransactionServiceInitializer::new(
                TransactionServiceConfig::new(
                    broadcast_monitoring_timeout,
                                              chain_monitoring_timeout,
                                              direct_send_timeout,
                                              broadcast_send_timeout),
                subscription_factory,
                transaction_db,
                wallet_comms.node_identity(),
                factories,network
            ))
            .finish()
            .await?;

        let comms = handles
            .take_handle::<UnspawnedCommsNode>()
            .expect("UnspawnedCommsNode not registered");
        let comms = initialization::spawn_comms_from_transport_type(comms, transport_type).await?;

        // Save final node identity after comms has initialized. This is required because the public_address can be
        // changed by comms during initialization when using tor.
        identity_management::save_as_json(&config.identity_file, &*comms.node_identity())
            .map_err(|e| anyhow!("Failed to save node identity: {:?}", e))?;
        if let Some(hs) = comms.hidden_service() {
            identity_management::save_as_json(&config.tor_identity_file, hs.tor_identity())
                .map_err(|e| anyhow!("Failed to save tor identity: {:?}", e))?;
        }

        Ok((comms, handles))
    }

    fn create_comms_config(&self) -> CommsConfig {
        CommsConfig {
            user_agent: format!("tari/wallet/{}", env!("CARGO_PKG_VERSION")),
            node_identity: self.node_identity.clone(),
            transport_type: self.create_transport_type(),
            datastore_path: self.config.wallet_peer_db_path.clone(),
            peer_database_name: "peers".to_string(),
            max_concurrent_inbound_tasks: 100,
            outbound_buffer_size: 100,
            // TODO - make this configurable
            dht: DhtConfig {
                database_url: DbConnectionUrl::File(self.config.data_dir.join("dht-wallet.db")),
                auto_join: true,
                ..Default::default()
            },
            allow_test_addresses: false,
            listener_liveness_allowlist_cidrs: Vec::new(),
            listener_liveness_max_sessions: 0,
        }
    }

    /// Creates a transport type for the base node's wallet using the provided configuration
    ///
    /// ##Returns
    /// TransportType based on the configuration
    pub fn create_transport_type(&self) -> TransportType {
        let config = &self.config;
        debug!(
            target: LOG_TARGET,
            "Wallet transport is set to '{:?}'", config.comms_transport
        );

        let add_to_port = |addr: Multiaddr, n| -> Multiaddr {
            addr.iter()
                .map(|p| match p {
                    Protocol::Tcp(port) => Protocol::Tcp(port + n),
                    p => p,
                })
                .collect()
        };

        match config.comms_transport.clone() {
            CommsTransport::Tcp {
                listener_address,
                tor_socks_address,
                tor_socks_auth,
            } => TransportType::Tcp {
                listener_address: add_to_port(listener_address, 1),
                tor_socks_config: tor_socks_address.map(|proxy_address| SocksConfig {
                    proxy_address,
                    authentication: tor_socks_auth
                        .map(utilities::convert_socks_authentication)
                        .unwrap_or_default(),
                }),
            },
            CommsTransport::TorHiddenService {
                control_server_address,
                socks_address_override,
                auth,
                ..
            } => {
                // The wallet should always use an OS-assigned forwarding port and an onion port number of 18101
                // to ensure that different wallet implementations cannot be differentiated by their port.
                let port_mapping = (18101u16, "127.0.0.1:0".parse::<SocketAddr>().unwrap()).into();

                let tor_identity_path = &config.wallet_tor_identity_file;
                let identity = if tor_identity_path.exists() {
                    // If this fails, we can just use another address
                    identity_management::load_from_json::<_, TorIdentity>(&tor_identity_path).ok()
                } else {
                    None
                };
                info!(
                    target: LOG_TARGET,
                    "Wallet tor identity at path '{}' {:?}",
                    tor_identity_path.to_string_lossy(),
                    identity
                        .as_ref()
                        .map(|ident| format!("loaded for address '{}.onion'", ident.service_id))
                        .or_else(|| Some("not found".to_string()))
                        .unwrap()
                );

                TransportType::Tor(TorConfig {
                    control_server_addr: control_server_address,
                    control_server_auth: {
                        match auth {
                            TorControlAuthentication::None => tor::Authentication::None,
                            TorControlAuthentication::Password(password) => {
                                tor::Authentication::HashedPassword(password)
                            },
                        }
                    },
                    identity: identity.map(Box::new),
                    port_mapping,
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
                    authentication: utilities::convert_socks_authentication(auth),
                },
                listener_address: add_to_port(listener_address, 1),
            },
        }
    }
}
