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

use std::{marker::PhantomData, str::FromStr, time::Instant};

use futures::future;
use log::*;
use tari_common::{
    configuration::Network,
    exit_codes::{ExitCode, ExitError},
};
use tari_network::{
    identity,
    multiaddr::multiaddr,
    MessageSpec,
    MessagingMode,
    NetworkError,
    NetworkHandle,
    OutboundMessaging,
    ReachabilityMode,
    SwarmConfig,
};
use tari_service_framework::{async_trait, ServiceInitializationError, ServiceInitializer, ServiceInitializerContext};
use tari_shutdown::ShutdownSignal;
use thiserror::Error;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    config::{P2pConfig, PeerSeedsConfig},
    connector::InboundMessaging,
    peer_seeds::{DnsSeedResolver, SeedPeer},
};

const LOG_TARGET: &str = "p2p::initialization";

#[derive(Debug, Error)]
pub enum CommsInitializationError {
    #[error("Invalid liveness CIDRs error: `{0}`")]
    InvalidLivenessCidrs(String),
    #[error("Could not add seed peer: `{0}`")]
    FailedToAddSeedPeer(NetworkError),
    #[error("Network error: `{0}`")]
    NetworkError(#[from] NetworkError),
    #[error("Cannot acquire exclusive file lock, another instance of the application is already running")]
    CannotAcquireFileLock,
    #[error("Invalid tor forward address: `{0}`")]
    InvalidTorForwardAddress(std::io::Error),
    #[error("IO Error: `{0}`")]
    IoError(#[from] std::io::Error),
}

impl CommsInitializationError {
    pub fn to_exit_error(&self) -> ExitError {
        ExitError::new(ExitCode::NetworkError, self)
    }
}

/// Initialize Tari Comms configured for tests
pub async fn initialize_local_test_comms<TMsg>(
    identity: identity::Keypair,
    seed_peers: Vec<SeedPeer>,
    shutdown_signal: ShutdownSignal,
) -> Result<
    (
        NetworkHandle,
        OutboundMessaging<TMsg>,
        InboundMessaging<TMsg>,
        JoinHandle<Result<(), NetworkError>>,
    ),
    CommsInitializationError,
>
where
    TMsg: MessageSpec + 'static,
    TMsg::Message: prost::Message + Default + Clone + 'static,
{
    let config = tari_network::Config {
        listener_addrs: vec![multiaddr![Ip4([0, 0, 0, 0]), Tcp(0u16)]],
        swarm: SwarmConfig {
            protocol_version: format!("/tari/{}/0.0.1", Network::LocalNet).parse().unwrap(),
            user_agent: "/tari/test/0.0.1".to_string(),
            enable_mdns: false,
            enable_relay: false,
            ..Default::default()
        },
        reachability_mode: ReachabilityMode::Private,
        announce: true,
        ..Default::default()
    };

    let (tx_messages, rx_messages) = mpsc::unbounded_channel();

    let (network, outbound_messaging, join_handle) = tari_network::spawn(
        identity,
        MessagingMode::Enabled { tx_messages },
        config,
        seed_peers.into_iter().map(Into::into).collect(),
        shutdown_signal,
    )?;

    let inbound_messaging = InboundMessaging::new(rx_messages);

    Ok((network, outbound_messaging, inbound_messaging, join_handle))
}

pub fn spawn_network<TMsg: MessageSpec>(
    identity: identity::Keypair,
    seed_peers: Vec<SeedPeer>,
    config: tari_network::Config,
    shutdown_signal: ShutdownSignal,
) -> Result<
    (
        NetworkHandle,
        OutboundMessaging<TMsg>,
        InboundMessaging<TMsg>,
        JoinHandle<Result<(), NetworkError>>,
    ),
    CommsInitializationError,
>
where
    TMsg: MessageSpec + 'static,
    TMsg::Message: prost::Message + Default + Clone + 'static,
{
    let (tx_messages, rx_messages) = mpsc::unbounded_channel();

    let (network, outbound_messaging, join_handle) = tari_network::spawn(
        identity,
        MessagingMode::Enabled { tx_messages },
        config,
        seed_peers.into_iter().map(Into::into).collect(),
        shutdown_signal,
    )?;

    let inbound_messaging = InboundMessaging::new(rx_messages);

    Ok((network, outbound_messaging, inbound_messaging, join_handle))
}

/// Adds seed peers to the list of known peers
pub async fn add_seed_peers(
    network: NetworkHandle,
    identity: &identity::Keypair,
    peers: Vec<SeedPeer>,
) -> Result<(), CommsInitializationError> {
    for peer in peers {
        if identity.public().is_eq_sr25519(&peer.public_key) {
            continue;
        }

        debug!(target: LOG_TARGET, "Adding seed peer [{}]", peer);
        network
            .add_peer(peer.into())
            .await
            .map_err(CommsInitializationError::FailedToAddSeedPeer)?;
    }
    Ok(())
}

pub struct P2pInitializer<TMsg> {
    config: P2pConfig,
    user_agent: String,
    seed_config: PeerSeedsConfig,
    network: Network,
    identity: identity::Keypair,
    _mgs_spec: PhantomData<TMsg>,
}

impl<TMsg> P2pInitializer<TMsg>
where
    TMsg: MessageSpec + 'static,
    TMsg::Message: prost::Message + Default + Clone + 'static,
{
    pub fn new(
        config: P2pConfig,
        user_agent: String,
        seed_config: PeerSeedsConfig,
        network: Network,
        identity: identity::Keypair,
    ) -> Self {
        Self {
            config,
            user_agent,
            seed_config,
            network,
            identity,
            _mgs_spec: PhantomData,
        }
    }

    fn try_parse_seed_peers(peer_seeds_str: &[String]) -> Result<Vec<SeedPeer>, ServiceInitializationError> {
        peer_seeds_str
            .iter()
            .map(|s| SeedPeer::from_str(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn try_resolve_dns_seeds(config: &PeerSeedsConfig) -> Result<Vec<SeedPeer>, ServiceInitializationError> {
        if config.dns_seeds.is_empty() {
            debug!(target: LOG_TARGET, "No DNS Seeds configured");
            return Ok(Vec::new());
        }

        debug!(
            target: LOG_TARGET,
            "Resolving DNS seeds (NS:{}, addresses: {})...",
            config.dns_seeds_name_server,
            config
                .dns_seeds
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<String>>()
                .join(",")
        );
        let start = Instant::now();

        let resolver = if config.dns_seeds_use_dnssec {
            debug!(
                target: LOG_TARGET,
                "Using {} to resolve DNS seeds. DNSSEC is enabled", config.dns_seeds_name_server
            );
            DnsSeedResolver::connect_secure(config.dns_seeds_name_server.clone()).await?
        } else {
            debug!(
                target: LOG_TARGET,
                "Using {} to resolve DNS seeds. DNSSEC is disabled", config.dns_seeds_name_server
            );
            DnsSeedResolver::connect(config.dns_seeds_name_server.clone()).await?
        };
        let resolving = config.dns_seeds.iter().map(|addr| {
            let mut resolver = resolver.clone();
            async move { (resolver.resolve(addr).await, addr) }
        });

        let peers = future::join_all(resolving)
            .await
            .into_iter()
            // Log and ignore errors
            .filter_map(|(result, addr)| match result {
                Ok(peers) => {
                    debug!(
                        target: LOG_TARGET,
                        "Found {} peer(s) from `{}` in {:.0?}",
                        peers.len(),
                        addr,
                        start.elapsed()
                    );
                    Some(peers)
                }
                Err(err) => {
                    warn!(target: LOG_TARGET, "DNS seed `{}` failed to resolve: {}", addr, err);
                    None
                }
            })
            .flatten()
            .collect();

        Ok(peers)
    }
}

#[async_trait]
impl<TMsg> ServiceInitializer for P2pInitializer<TMsg>
where
    TMsg: MessageSpec + Send + Sync + 'static,
    TMsg::Message: prost::Message + Default + Clone + Send + Sync + 'static,
{
    async fn initialize(&mut self, context: ServiceInitializerContext) -> Result<(), ServiceInitializationError> {
        debug!(target: LOG_TARGET, "Initializing P2P");
        let seed_peers = Self::try_parse_seed_peers(&self.seed_config.peer_seeds)?;

        let dns_peers = Self::try_resolve_dns_seeds(&self.seed_config)
            .await
            .unwrap_or_else(|err| {
                warn!(target: LOG_TARGET, "Failed to resolve DNS seeds: {}", err);
                Vec::new()
            });

        let config = tari_network::Config {
            swarm: SwarmConfig {
                protocol_version: format!("/minotari/{}/1.0.0", self.network.as_key_str()).parse()?,
                user_agent: self.user_agent.clone(),
                enable_mdns: self.config.enable_mdns,
                enable_relay: self.config.enable_relay,
                ..Default::default()
            },
            listener_addrs: self.config.listen_addresses.clone(),
            reachability_mode: self.config.reachability_mode,
            announce: true,
            check_connections_interval: Default::default(),
            known_local_public_address: self.config.public_addresses.to_vec(),
        };

        let shutdown = context.get_shutdown_signal();
        let (network, outbound_messaging, inbound_messaging, _join_handle) = spawn_network::<TMsg>(
            self.identity.clone(),
            seed_peers.into_iter().chain(dns_peers).collect(),
            config,
            shutdown,
        )?;

        context.register_handle(network);
        context.register_handle(outbound_messaging);
        context.register_handle(inbound_messaging);
        debug!(target: LOG_TARGET, "P2P Initialized");
        Ok(())
    }
}
