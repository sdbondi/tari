//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use libp2p::{identity::Keypair, PeerId};
use tari_shutdown::ShutdownSignal;
use tari_swarm::{is_supported_multiaddr, messaging, messaging::prost::ProstCodec};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::{
    message::MessageSpec,
    messaging::OutboundMessaging,
    worker::NetworkingWorker,
    NetworkError,
    NetworkHandle,
    Peer,
};

pub fn spawn<TMsg>(
    identity: Keypair,
    messaging_mode: MessagingMode<TMsg>,
    mut config: crate::Config,
    seed_peers: Vec<Peer>,
    shutdown_signal: ShutdownSignal,
) -> Result<
    (
        NetworkHandle,
        OutboundMessaging<TMsg>,
        JoinHandle<Result<(), NetworkError>>,
    ),
    NetworkError,
>
where
    TMsg: MessageSpec + 'static,
    TMsg::Message: messaging::prost::Message + Default + Clone + 'static,
    TMsg: MessageSpec,
{
    for peer in &seed_peers {
        for addr in &peer.addresses {
            if !is_supported_multiaddr(addr) {
                return Err(NetworkError::UnsupportedSeedPeerMultiaddr { address: addr.clone() });
            }
        }
    }

    config.swarm.enable_relay = config.swarm.enable_relay || !config.reachability_mode.is_private();
    config.swarm.enable_messaging = messaging_mode.is_enabled();
    let swarm =
        tari_swarm::create_swarm::<ProstCodec<TMsg::Message>>(identity.clone(), HashSet::new(), config.swarm.clone())?;
    let local_peer_id = *swarm.local_peer_id();
    let (tx_requests, rx_requests) = mpsc::channel(1);
    let (tx_msg_requests, rx_msg_requests) = mpsc::channel(100);
    let (tx_events, _) = broadcast::channel(100);
    let handle = tokio::spawn(
        NetworkingWorker::<TMsg>::new(
            identity,
            rx_requests,
            rx_msg_requests,
            tx_events.clone(),
            messaging_mode,
            swarm,
            config,
            seed_peers,
            vec![],
            shutdown_signal,
        )
        .run(),
    );
    Ok((
        NetworkHandle::new(local_peer_id, tx_requests, tx_events),
        OutboundMessaging::new(tx_msg_requests),
        handle,
    ))
}

pub enum MessagingMode<TMsg: MessageSpec> {
    Enabled {
        tx_messages: mpsc::UnboundedSender<(PeerId, TMsg::Message)>,
    },
    Disabled,
}

impl<TMsg: MessageSpec> MessagingMode<TMsg> {
    pub fn is_enabled(&self) -> bool {
        matches!(self, MessagingMode::Enabled { .. })
    }
}

impl<TMsg: MessageSpec> MessagingMode<TMsg> {
    pub fn send_message(
        &self,
        peer_id: PeerId,
        msg: TMsg::Message,
    ) -> Result<(), mpsc::error::SendError<(PeerId, TMsg::Message)>> {
        if let MessagingMode::Enabled { tx_messages, .. } = self {
            tx_messages.send((peer_id, msg))?;
        }
        Ok(())
    }
}
