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

//! # MemoryNet
//!
//! This example runs a small in-memory network.
//! It's primary purpose is to test and debug the behaviour of the DHT.
//!
//! The following happens:
//! 1. A single "seed node", `NUM_NODES` "base nodes" and `NUM_WALLETS` "wallets" are generated and started.
//! 1. All "base nodes" join the network via the "seed node"
//! 1. All "wallets" join the network via a random "base node"
//! 1. The first "wallet" in the list attempts to discover the last "wallet" in the list
//!
//! The suggested way to run this is:
//!
//! `RUST_BACKTRACE=1 RUST_LOG=trace cargo run --example memorynet 2> /tmp/debug.log`

// Size of network
const NUM_NODES: usize = 40;
// Must be at least 2
const NUM_WALLETS: usize = 6;
const QUIET_MODE: bool = true;
/// Number of neighbouring nodes each node should include in the connection pool
const NUM_NEIGHBOURING_NODES: usize = 8;
/// Number of randomly-selected nodes each node should include in the connection pool
const NUM_RANDOM_NODES: usize = 4;
/// The number of messages that should be propagated out
const PROPAGATION_FACTOR: usize = 4;

mod memory_net;

use futures::{channel::mpsc, future, StreamExt};
use lazy_static::lazy_static;
use memory_net::DrainBurst;
use prettytable::{cell, row, Table};
use rand::{rngs::OsRng, Rng};
use std::{
    collections::HashMap,
    fmt,
    iter::repeat_with,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tari_comms::{
    backoff::ConstantBackoff,
    connection_manager::ConnectionDirection,
    connectivity::ConnectivitySelection,
    peer_manager::{NodeId, NodeIdentity, Peer, PeerFeatures, PeerStorage},
    pipeline,
    pipeline::SinkService,
    protocol::messaging::MessagingEvent,
    transports::MemoryTransport,
    types::CommsDatabase,
    CommsBuilder,
    CommsNode,
    ConnectionManagerEvent,
    PeerConnection,
};
use tari_comms_dht::{
    domain_message::OutboundDomainMessage,
    envelope::NodeDestination,
    inbound::DecryptedDhtMessage,
    outbound::OutboundEncryption,
    Dht,
    DhtBuilder,
    DhtConfig,
};
use tari_storage::{
    lmdb_store::{LMDBBuilder, LMDBConfig},
    LMDBWrapper,
};
use tari_test_utils::{paths::create_temporary_data_path, random};
use tokio::{runtime, task, time};
use tower::ServiceBuilder;

type MessagingEventRx = mpsc::UnboundedReceiver<(NodeId, NodeId)>;
type MessagingEventTx = mpsc::UnboundedSender<(NodeId, NodeId)>;

macro_rules! banner {
    ($($arg: tt)*) => {
        println!();
        println!("----------------------------------------------------------");
        println!($($arg)*);
        println!("----------------------------------------------------------");
        println!();
    }
}

const NAMES: &[&str] = &[
    "Alice", "Bob", "Carol", "Charlie", "Dave", "Eve", "Isaac", "Ivan", "Justin", "Mallory", "Marvin", "Mallet",
    "Matilda", "Oscar", "Pat", "Peggy", "Vanna", "Plod", "Steve", "Trent", "Trudy", "Walter", "Zoe",
];

lazy_static! {
    static ref NAME_MAP: Mutex<HashMap<NodeId, String>> = Mutex::new(HashMap::new());
    static ref NAME_POS: Mutex<usize> = Mutex::new(0);
}

fn register_name(node_id: NodeId, name: String) {
    NAME_MAP.lock().unwrap().insert(node_id, name);
}

fn get_name(node_id: &NodeId) -> String {
    NAME_MAP
        .lock()
        .unwrap()
        .get(node_id)
        .map(|name| format!("{} ({})", name, node_id.short_str()))
        .unwrap_or_else(|| format!("NoName ({})", node_id.short_str()))
}

fn get_short_name(node_id: &NodeId) -> String {
    NAME_MAP
        .lock()
        .unwrap()
        .get(node_id)
        .map(|name| format!("{}", name))
        .unwrap_or_else(|| format!("NoName ({})", node_id.short_str()))
}

fn get_next_name() -> String {
    let pos = {
        let mut i = NAME_POS.lock().unwrap();
        *i = *i + 1;
        *i
    };
    if pos > NAMES.len() {
        format!("Node{}", pos - NAMES.len())
    } else {
        NAMES[pos - 1].to_owned()
    }
}

#[tokio_macros::main]
async fn main() {
    env_logger::init();

    banner!(
        "Bringing up virtual network consisting of a seed node, {} nodes and {} wallets",
        NUM_NODES,
        NUM_WALLETS
    );

    let (messaging_events_tx, mut messaging_events_rx) = mpsc::unbounded();

    let seed_node = make_node(PeerFeatures::COMMUNICATION_NODE, None, messaging_events_tx.clone()).await;

    let mut nodes = future::join_all(
        repeat_with(|| {
            make_node(
                PeerFeatures::COMMUNICATION_NODE,
                Some(seed_node.to_peer()),
                messaging_events_tx.clone(),
            )
        })
        .take(NUM_NODES),
    )
    .await;

    let mut wallets = future::join_all(
        repeat_with(|| {
            make_node(
                PeerFeatures::COMMUNICATION_CLIENT,
                // Some(seed_node.to_peer()),
                Some(nodes[OsRng.gen_range(0, NUM_NODES - 1)].to_peer()),
                messaging_events_tx.clone(),
            )
        })
        .take(NUM_WALLETS),
    )
    .await;

    // Every node knows about every other node/client - uncomment this if you want to see the effect of "perfect network
    // knowledge" on the network.
    // for n in &nodes {
    //     for ni in &nodes {
    //         if n.node_identity().node_id() != ni.node_identity().node_id() {
    //             n.comms
    //                 .peer_manager()
    //                 .add_peer(ni.node_identity().to_peer())
    //                 .await
    //                 .unwrap();
    //         }
    //     }
    //     for ni in &wallets {
    //         n.comms
    //             .peer_manager()
    //             .add_peer(ni.node_identity().to_peer())
    //             .await
    //             .unwrap();
    //     }
    // }

    log::info!("------------------------------- BASE NODE JOIN -------------------------------");
    for node in nodes.iter_mut() {
        println!(
            "Node '{}' is joining the network via the seed node '{}'",
            node, seed_node
        );
        node.comms
            .connectivity()
            .wait_for_connectivity(Duration::from_secs(10))
            .await
            .unwrap();

        node.dht.dht_requester().send_join().await.unwrap();
    }

    take_a_break().await;

    // peer_list_summary(&nodes).await;

    banner!(
        "Now, {} wallets are going to join from a random base node.",
        NUM_WALLETS
    );

    log::info!("------------------------------- WALLET JOIN -------------------------------");
    for wallet in wallets.iter_mut() {
        println!(
            "Wallet '{}' is joining the network via node '{}'",
            wallet,
            get_name(&wallet.seed_peer.as_ref().unwrap().node_id)
        );
        wallet
            .comms
            .connectivity()
            .wait_for_connectivity(Duration::from_secs(10))
            .await
            .unwrap();

        wallet.dht.dht_requester().send_join().await.unwrap();
    }

    log::info!("------------------------------- BASE NODE JOIN -------------------------------");
    for node in nodes.iter_mut() {
        println!(
            "Node '{}' is joining the network via the seed node '{}'",
            node, seed_node
        );
        node.comms
            .connectivity()
            .wait_for_connectivity(Duration::from_secs(10))
            .await
            .unwrap();

        node.dht.dht_requester().send_join().await.unwrap();
    }

    // take_a_break().await;
    let mut total_messages = 0;
    total_messages += drain_messaging_events(&mut messaging_events_rx, false).await;

    network_peer_list_stats(&nodes, &nodes).await;
    network_peer_list_stats(&nodes, &wallets).await;
    network_connectivity_stats(&nodes, &wallets).await;

    {
        let count = seed_node.comms.peer_manager().count().await;
        let num_connections = seed_node
            .comms
            .connection_manager()
            .get_num_active_connections()
            .await
            .unwrap();
        println!("Seed node knows {} peers ({} connections)", count, num_connections);
    }

    take_a_break().await;

    // log::info!("------------------------------- DISCOVERY -------------------------------");
    // total_messages += discovery(&wallets, &mut messaging_events_rx).await;
    //
    // total_messages += drain_messaging_events(&mut messaging_events_rx, false).await;
    //
    // log::info!("------------------------------- SAF/DIRECTED PROPAGATION -------------------------------");
    // for _ in 0..5 {
    //     let random_wallet = wallets.remove(OsRng.gen_range(0, wallets.len() - 1));
    //     let (num_msgs, random_wallet) = do_store_and_forward_message_propagation(
    //         random_wallet,
    //         &wallets,
    //         &nodes,
    //         messaging_events_tx.clone(),
    //         &mut messaging_events_rx,
    //     )
    //     .await;
    //     total_messages += num_msgs;
    //     // Put the wallet back
    //     wallets.push(random_wallet);
    // }

    let num_nodes = nodes.len();
    log::info!("------------------------------- PROPAGATION -------------------------------");
    let failures = do_network_wide_propagation(&mut nodes, OsRng.gen_range(0, num_nodes - 1)).await;
    total_messages += drain_messaging_events(&mut messaging_events_rx, false).await;
    log::info!("------------------------------- PROPAGATION -------------------------------");
    let next_idx = failures
        .first()
        .map(|v| *v)
        .unwrap_or_else(|| OsRng.gen_range(0, num_nodes - 1));
    let failures = do_network_wide_propagation(&mut nodes, next_idx).await;
    total_messages += drain_messaging_events(&mut messaging_events_rx, false).await;
    log::info!("------------------------------- PROPAGATION -------------------------------");
    let next_idx = failures
        .first()
        .map(|v| *v)
        .unwrap_or_else(|| OsRng.gen_range(0, num_nodes - 1));
    let failures = do_network_wide_propagation(&mut nodes, next_idx).await;
    total_messages += drain_messaging_events(&mut messaging_events_rx, false).await;
    log::info!("------------------------------- PROPAGATION -------------------------------");
    let next_idx = failures
        .first()
        .map(|v| *v)
        .unwrap_or_else(|| OsRng.gen_range(0, num_nodes - 1));
    do_network_wide_propagation(&mut nodes, next_idx).await;
    total_messages += drain_messaging_events(&mut messaging_events_rx, false).await;

    println!("{} messages sent in total across the network", total_messages);

    network_peer_list_stats(&nodes, &wallets).await;
    network_connectivity_stats(&nodes, &wallets).await;

    banner!("That's it folks! Network is shutting down...");
    log::info!("------------------------------- SHUTDOWN -------------------------------");

    shutdown_all(nodes).await;
    shutdown_all(wallets).await;
}

async fn shutdown_all(nodes: Vec<TestNode>) {
    let tasks = nodes.into_iter().map(|node| node.comms.shutdown());
    future::join_all(tasks).await;
}

async fn discovery(wallets: &[TestNode], messaging_events_rx: &mut MessagingEventRx) -> usize {
    let mut successes = 0;
    let mut total_messages = 0;
    let mut total_time = Duration::from_secs(0);
    for i in 0..wallets.len() - 1 {
        let wallet1 = wallets.get(i).unwrap();
        let wallet2 = wallets.get(i + 1).unwrap();

        banner!("🌎 '{}' is going to try discover '{}'.", wallet1, wallet2);

        if !QUIET_MODE {
            peer_list_summary(&[wallet1, wallet2]).await;
        }

        let start = Instant::now();
        let discovery_result = wallet1
            .dht
            .discovery_service_requester()
            .discover_peer(
                Box::new(wallet2.node_identity().public_key().clone()),
                wallet2.node_identity().node_id().clone().into(),
            )
            .await;

        match discovery_result {
            Ok(peer) => {
                successes += 1;
                total_time += start.elapsed();
                banner!(
                    "⚡️🎉😎 '{}' discovered peer '{}' ({}) in {:.2?}",
                    wallet1,
                    get_name(&peer.node_id),
                    peer,
                    start.elapsed()
                );

                time::delay_for(Duration::from_secs(5)).await;
                total_messages += drain_messaging_events(messaging_events_rx, false).await;
            },
            Err(err) => {
                banner!(
                    "💩 '{}' failed to discover '{}' after {:.2?} because '{}'",
                    wallet1,
                    wallet2,
                    start.elapsed(),
                    err
                );

                time::delay_for(Duration::from_secs(5)).await;
                total_messages += drain_messaging_events(messaging_events_rx, false).await;
            },
        }
    }

    banner!(
        "✨ The set of discoveries succeeded {} out of {} times and took a total of {:.1}s with {} messages sent.",
        successes,
        wallets.len() - 1,
        total_time.as_secs_f32(),
        total_messages
    );
    total_messages
}

async fn peer_list_summary<'a, I: IntoIterator<Item = T>, T: AsRef<TestNode>>(network: I) {
    for node in network {
        let node_identity = node.as_ref().comms.node_identity();
        let peers = node
            .as_ref()
            .comms
            .peer_manager()
            .closest_peers(node_identity.node_id(), 10, &[], None)
            .await
            .unwrap();
        let mut table = Table::new();
        table.add_row(row![
            format!("{} closest peers (MAX: 10)", node.as_ref()),
            "Distance".to_string(),
            "Kind",
        ]);
        table.add_empty_row();
        for peer in peers {
            table.add_row(row![
                get_name(&peer.node_id),
                node_identity.node_id().distance(&peer.node_id),
                if peer.features.contains(PeerFeatures::COMMUNICATION_NODE) {
                    "BaseNode"
                } else {
                    "Wallet"
                }
            ]);
        }
        table.printstd();
        println!();
    }
}

async fn network_peer_list_stats(nodes: &[TestNode], wallets: &[TestNode]) {
    let mut stats = HashMap::<String, usize>::with_capacity(wallets.len());
    for wallet in wallets {
        let mut num_known = 0;
        for node in nodes {
            if node
                .comms
                .peer_manager()
                .exists_node_id(wallet.node_identity().node_id())
                .await
            {
                num_known += 1;
            }
        }
        stats.insert(get_name(wallet.node_identity().node_id()), num_known);
    }

    let mut avg = Vec::with_capacity(wallets.len());
    for (n, v) in stats {
        let perc = v as f32 / nodes.len() as f32;
        avg.push(perc);
        println!(
            "{} is known by {} out of {} nodes ({:.2}%)",
            n,
            v,
            nodes.len(),
            perc * 100.0
        );
    }
    println!(
        "Average {:.2}%",
        avg.into_iter().sum::<f32>() / wallets.len() as f32 * 100.0
    );
}

async fn network_connectivity_stats(nodes: &[TestNode], wallets: &[TestNode]) {
    async fn display(nodes: &[TestNode]) -> (usize, usize) {
        let mut total = 0;
        let mut avg = Vec::new();
        for node in nodes {
            let conns = node.comms.connection_manager().get_active_connections().await.unwrap();
            total += conns.len();
            avg.push(conns.len());

            println!("{} connected to {} nodes", node, conns.len());
            if !QUIET_MODE {
                for c in conns {
                    println!("  {} ({})", get_name(c.peer_node_id()), c.direction());
                }
            }
        }
        (total, avg.into_iter().sum())
    }
    let (mut total, mut avg) = display(nodes).await;
    let (t, a) = display(wallets).await;
    total += t;
    avg += a;
    println!(
        "{} total connections on the network. ({} per node on average)",
        total,
        avg / (wallets.len() + nodes.len())
    );
}

async fn do_network_wide_propagation(nodes: &mut [TestNode], node_idx: usize) -> Vec<usize> {
    let random_node = &nodes[node_idx];
    let random_node_id = random_node.comms.node_identity().node_id().clone();
    let message = format!("This is something you're all interested in (from {})!", random_node);

    banner!("🌎 {} is going to broadcast a message to the network", random_node);
    let send_states = random_node
        .dht
        .outbound_requester()
        .broadcast(
            NodeDestination::Unknown,
            OutboundEncryption::None,
            vec![],
            OutboundDomainMessage::new(0i32, message),
        )
        .await
        .unwrap();
    let num_connections = random_node
        .comms
        .connection_manager()
        .get_num_active_connections()
        .await
        .unwrap();
    let (success, failed) = send_states.wait_all().await;
    println!(
        "🦠 {} broadcast to {}/{} peer(s) ({} connection(s))",
        random_node.name,
        success.len(),
        success.len() + failed.len(),
        num_connections
    );

    let start_global = Instant::now();
    // Spawn task for each peer that will read the message and propagate it on
    let tasks = nodes.into_iter().enumerate().map(|(idx, node)| {
        let mut outbound_req = node.dht.outbound_requester();
        let mut conn_man = node.comms.connection_manager();
        let mut inbound_msgs = node.inbound_msgs.take().unwrap();
        let start = Instant::now();
        let start_global = start_global.clone();
        let node_name = node.name.clone();
        let is_subject = node.comms.node_identity().node_id() == &random_node_id;

        task::spawn(async move {
            if is_subject {
                return (idx, inbound_msgs, false);
            }

            let result = time::timeout(Duration::from_secs(10), inbound_msgs.next()).await;
            let mut is_success = false;
            match result {
                Ok(Some(msg)) => {
                    let public_msg = msg
                        .decryption_result
                        .unwrap()
                        .decode_part::<String>(1)
                        .unwrap()
                        .unwrap();
                    println!(
                        "📬 {} got public message '{}' (t={:.0?})",
                        node_name,
                        public_msg,
                        start_global.elapsed()
                    );
                    is_success = true;
                    let send_states = outbound_req
                        .broadcast(
                            NodeDestination::Unknown,
                            OutboundEncryption::None,
                            vec![msg.source_peer.node_id.clone()],
                            OutboundDomainMessage::new(0i32, public_msg),
                        )
                        .await
                        .unwrap();
                    let num_connections = conn_man.get_num_active_connections().await.unwrap();
                    let (success, failed) = send_states.wait_all().await;
                    println!(
                        "🦠 {} propagated to {}/{} peer(s) ({} connection(s))",
                        node_name,
                        success.len(),
                        success.len() + failed.len(),
                        num_connections
                    );
                },
                Err(_) | Ok(None) => {
                    banner!(
                        "💩 {} failed to receive network message after {:.2?}",
                        node_name,
                        start.elapsed(),
                    );
                },
            }

            (idx, inbound_msgs, is_success)
        })
    });

    // Put the ims_rxs back
    let results = future::join_all(tasks).await;
    let mut num_successes = 0;
    let mut failures = Vec::new();
    for result in results {
        let (idx, inbound_msgs, is_success) = result.unwrap();
        nodes[idx].inbound_msgs = Some(inbound_msgs);
        if is_success {
            num_successes += 1;
        } else {
            failures.push(idx);
        }
    }

    banner!(
        "🙌 Finished propagation test. {} out of {} nodes received the message",
        num_successes,
        nodes.len() - 1
    );
    failures
}

async fn do_store_and_forward_message_propagation(
    wallet: TestNode,
    wallets: &[TestNode],
    nodes: &[TestNode],
    messaging_tx: MessagingEventTx,
    messaging_rx: &mut MessagingEventRx,
) -> (usize, TestNode)
{
    banner!(
        "{} chosen at random to be receive messages from other nodes using store and forward",
        wallet,
    );
    let wallets_peers = wallet.comms.peer_manager().all().await.unwrap();
    let node_identity = wallet.comms.node_identity().clone();

    let neighbours = wallet
        .comms
        .connectivity()
        .select_connections(ConnectivitySelection::closest_to(
            wallet.node_identity().node_id().clone(),
            NUM_NEIGHBOURING_NODES,
            vec![],
        ))
        .await
        .unwrap()
        .into_iter()
        .filter_map(
            // If a node is not found in the node list it must be the seed node - should probably assert that this is
            // the case
            |p| nodes.iter().find(|n| n.node_identity().node_id() == p.peer_node_id()),
        )
        .collect::<Vec<_>>();

    let neighbour_subs = neighbours
        .iter()
        .map(|n| n.comms.subscribe_messaging_events())
        .collect::<Vec<_>>();

    banner!(
        "{} has {} neighbours ({})",
        wallet,
        neighbours.len(),
        neighbours
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    banner!("😴 {} is going offline", wallet);
    wallet.comms.shutdown().await;

    banner!(
        "🎤 All other wallets are going to attempt to broadcast messages to {} ({})",
        get_name(node_identity.node_id()),
        node_identity.public_key(),
    );

    let start = Instant::now();
    for wallet in wallets {
        let secret_message = format!("My name is wiki wiki {}", wallet);
        let send_states = wallet
            .dht
            .outbound_requester()
            .broadcast(
                node_identity.node_id().clone().into(),
                OutboundEncryption::EncryptFor(Box::new(node_identity.public_key().clone())),
                vec![],
                OutboundDomainMessage::new(123i32, secret_message.clone()),
            )
            .await
            .unwrap();
        let (success, failed) = send_states.wait_all().await;
        println!(
            "{} sent {}/{} messages",
            wallet,
            success.len(),
            success.len() + failed.len(),
        );
    }

    for (idx, mut s) in neighbour_subs.into_iter().enumerate() {
        let neighbour = neighbours[idx].name.clone();
        task::spawn(async move {
            let msg = time::timeout(Duration::from_secs(2), s.next()).await;
            match msg {
                Ok(Some(Ok(evt))) => match &*evt {
                    MessagingEvent::MessageReceived(_, tag) => {
                        println!("{} received propagated SAF message ({})", neighbour, tag);
                    },
                    _ => {},
                },
                Ok(_) => {},
                Err(_) => println!("{} did not receive the SAF message", neighbour),
            }
        });
    }

    banner!("⏰ Waiting a few seconds for messages to propagate around the network...");
    time::delay_for(Duration::from_secs(5)).await;

    let mut total_messages = drain_messaging_events(messaging_rx, false).await;

    banner!("🤓 {} is coming back online", get_name(node_identity.node_id()));
    let (tx, ims_rx) = mpsc::channel(1);
    let (comms, dht) = setup_comms_dht(node_identity, create_peer_storage(wallets_peers), tx).await;
    let mut wallet = TestNode::new(comms, dht, None, ims_rx, messaging_tx);
    let mut connectivity = wallet.comms.connectivity();

    connectivity
        .wait_for_connectivity(Duration::from_secs(10))
        .await
        .unwrap();
    take_a_break().await;
    let connections = wallet
        .comms
        .connection_manager()
        .get_active_connections()
        .await
        .unwrap();
    println!(
        "{} has {} connections to {}",
        wallet,
        connections.len(),
        connections
            .iter()
            .map(|p| get_name(p.peer_node_id()))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut num_msgs = 0;
    loop {
        let result = time::timeout(Duration::from_secs(10), wallet.inbound_msgs.as_mut().unwrap().next()).await;
        num_msgs += 1;
        match result {
            Ok(msg) => {
                let msg = msg.unwrap();
                let secret_msg = msg
                    .decryption_result
                    .unwrap()
                    .decode_part::<String>(1)
                    .unwrap()
                    .unwrap();
                banner!(
                    "🎉 Wallet {} received propagated message '{}' from store and forward in {:.2?}",
                    wallet,
                    secret_msg,
                    start.elapsed()
                );
            },
            Err(err) => {
                banner!(
                    "💩 Failed to receive message after {:.0?} using store and forward '{}'",
                    start.elapsed(),
                    err
                );
            },
        };

        if num_msgs == wallets.len() {
            break;
        }
    }

    total_messages += drain_messaging_events(messaging_rx, false).await;

    (total_messages, wallet)
}

async fn drain_messaging_events(messaging_rx: &mut MessagingEventRx, show_logs: bool) -> usize {
    let drain_fut = DrainBurst::new(messaging_rx);
    if show_logs {
        let messages = drain_fut.await;
        let num_messages = messages.len();
        let mut node_id_buf = Vec::new();
        let mut last_from_node = None;
        for (from_node, to_node) in &messages {
            match &last_from_node {
                Some(node_id) if *node_id == from_node => {
                    node_id_buf.push(to_node);
                },
                Some(_) => {
                    println!(
                        "📨 {} sent {} messages to {}️",
                        get_short_name(last_from_node.take().unwrap()),
                        node_id_buf.len(),
                        node_id_buf.drain(..).map(get_short_name).collect::<Vec<_>>().join(", ")
                    );

                    last_from_node = Some(from_node);
                    node_id_buf.push(to_node)
                },
                None => {
                    last_from_node = Some(from_node);
                    node_id_buf.push(to_node)
                },
            }
        }
        println!("{} messages sent between nodes", num_messages);
        num_messages
    } else {
        let len = drain_fut.await.len();
        println!("📨 {} messages exchanged", len);
        len
    }
}

fn connection_manager_logger(
    node_id: NodeId,
) -> impl FnMut(Arc<ConnectionManagerEvent>) -> Arc<ConnectionManagerEvent> {
    let node_name = get_name(&node_id);
    move |event| {
        if QUIET_MODE {
            return event;
        }
        use ConnectionManagerEvent::*;
        print!("EVENT: ");
        match &*event {
            PeerConnected(conn) => match conn.direction() {
                ConnectionDirection::Inbound => {
                    // println!(
                    //     "'{}' got inbound connection from '{}'",
                    //     node_name,
                    //     get_name(conn.peer_node_id()),
                    // );
                },
                ConnectionDirection::Outbound => {
                    println!("'{}' connected to '{}'", node_name, get_name(conn.peer_node_id()),);
                },
            },
            PeerDisconnected(node_id) => {
                println!("'{}' disconnected from '{}'", get_name(node_id), node_name);
            },
            PeerConnectFailed(node_id, err) => {
                println!(
                    "'{}' failed to connect to '{}' because '{:?}'",
                    node_name,
                    get_name(node_id),
                    err
                );
            },
            PeerConnectWillClose(_, node_id, direction) => {
                println!(
                    "'{}' will disconnect {} connection to '{}'",
                    get_name(node_id),
                    direction,
                    node_name,
                );
            },
            PeerInboundConnectFailed(err) => {
                println!(
                    "'{}' failed to accept inbound connection because '{:?}'",
                    node_name, err
                );
            },
            Listening(_) | ListenFailed(_) => unreachable!(),
            NewInboundSubstream(node_id, protocol, _) => {
                println!(
                    "'{}' negotiated protocol '{}' to '{}'",
                    get_name(node_id),
                    String::from_utf8_lossy(protocol),
                    node_name
                );
            },
        }
        event
    }
}

struct TestNode {
    name: String,
    comms: CommsNode,
    seed_peer: Option<Peer>,
    dht: Dht,
    conn_man_events_rx: mpsc::Receiver<Arc<ConnectionManagerEvent>>,
    inbound_msgs: Option<mpsc::Receiver<DecryptedDhtMessage>>,
}

impl TestNode {
    pub fn new(
        comms: CommsNode,
        dht: Dht,
        seed_peer: Option<Peer>,
        ims_rx: mpsc::Receiver<DecryptedDhtMessage>,
        messaging_events_tx: MessagingEventTx,
    ) -> Self
    {
        let name = get_next_name();
        register_name(comms.node_identity().node_id().clone(), name.clone());

        let (conn_man_events_tx, events_rx) = mpsc::channel(100);
        Self::spawn_event_monitor(&comms, conn_man_events_tx, messaging_events_tx);

        Self {
            name,
            seed_peer,
            comms,
            dht,
            inbound_msgs: Some(ims_rx),
            conn_man_events_rx: events_rx,
        }
    }

    fn spawn_event_monitor(
        comms: &CommsNode,
        events_tx: mpsc::Sender<Arc<ConnectionManagerEvent>>,
        messaging_events_tx: MessagingEventTx,
    )
    {
        let conn_man_event_sub = comms.subscribe_connection_manager_events();
        let messaging_events = comms.subscribe_messaging_events();
        let executor = runtime::Handle::current();

        executor.spawn(
            conn_man_event_sub
                .filter(|r| future::ready(r.is_ok()))
                .map(Result::unwrap)
                .map(connection_manager_logger(comms.node_identity().node_id().clone()))
                .map(Ok)
                .forward(events_tx),
        );

        let node_id = comms.node_identity().node_id().clone();

        executor.spawn(
            messaging_events
                .filter(|r| future::ready(r.is_ok()))
                .map(Result::unwrap)
                .filter_map(move |event| {
                    use MessagingEvent::*;
                    future::ready(match &*event {
                        MessageReceived(peer_node_id, _) => Some((Clone::clone(&*peer_node_id), node_id.clone())),
                        _ => None,
                    })
                })
                .map(Ok)
                .forward(messaging_events_tx),
        );
    }

    #[inline]
    pub fn node_identity(&self) -> Arc<NodeIdentity> {
        self.comms.node_identity()
    }

    #[inline]
    pub fn to_peer(&self) -> Peer {
        self.comms.node_identity().to_peer()
    }

    #[allow(dead_code)]
    pub async fn expect_peer_connection(&mut self, node_id: &NodeId) -> Option<PeerConnection> {
        if let Some(conn) = self.comms.connectivity().get_connection(node_id.clone()).await.unwrap() {
            return Some(conn);
        }
        use ConnectionManagerEvent::*;
        loop {
            let event = time::timeout(Duration::from_secs(30), self.conn_man_events_rx.next())
                .await
                .ok()??;

            match &*event {
                PeerConnected(conn) if conn.peer_node_id() == node_id => {
                    break Some(conn.clone());
                },
                _ => {},
            }
        }
    }
}

impl AsRef<TestNode> for TestNode {
    fn as_ref(&self) -> &TestNode {
        self
    }
}

impl fmt::Display for TestNode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} ({})",
            self.name,
            self.comms.node_identity().node_id().short_str()
        )
    }
}

fn make_node_identity(features: PeerFeatures) -> Arc<NodeIdentity> {
    let port = MemoryTransport::acquire_next_memsocket_port();
    Arc::new(NodeIdentity::random(&mut OsRng, format!("/memory/{}", port).parse().unwrap(), features).unwrap())
}

fn create_peer_storage(peers: Vec<Peer>) -> CommsDatabase {
    let database_name = random::string(8);
    let datastore = LMDBBuilder::new()
        .set_path(create_temporary_data_path().to_str().unwrap())
        .set_env_config(LMDBConfig::default())
        .set_max_number_of_databases(1)
        .add_database(&database_name, lmdb_zero::db::CREATE)
        .build()
        .unwrap();

    let peer_database = datastore.get_handle(&database_name).unwrap();
    let peer_database = LMDBWrapper::new(Arc::new(peer_database));
    let mut storage = PeerStorage::new_indexed(peer_database).unwrap();
    for peer in peers {
        storage.add_peer(peer).unwrap();
    }

    storage.into()
}

async fn make_node(features: PeerFeatures, seed_peer: Option<Peer>, messaging_events_tx: MessagingEventTx) -> TestNode {
    let node_identity = make_node_identity(features);

    let (tx, ims_rx) = mpsc::channel(1);
    let (comms, dht) = setup_comms_dht(
        node_identity,
        create_peer_storage(seed_peer.clone().into_iter().collect()),
        tx,
    )
    .await;

    TestNode::new(comms, dht, seed_peer, ims_rx, messaging_events_tx)
}

async fn setup_comms_dht(
    node_identity: Arc<NodeIdentity>,
    storage: CommsDatabase,
    inbound_tx: mpsc::Sender<DecryptedDhtMessage>,
) -> (CommsNode, Dht)
{
    // Create inbound and outbound channels
    let (outbound_tx, outbound_rx) = mpsc::channel(10);

    let comms = CommsBuilder::new()
        .allow_test_addresses()
        // In this case the listener address and the public address are the same (/memory/...)
        .with_listener_address(node_identity.public_address())
        .with_transport(MemoryTransport)
        .with_node_identity(node_identity)
        .with_min_connectivity(0.3)
        .with_peer_storage(storage)
        .with_dial_backoff(ConstantBackoff::new(Duration::from_millis(1000)))
        .build()
        .unwrap();

    let dht = DhtBuilder::new(
        comms.node_identity(),
        comms.peer_manager(),
        outbound_tx,
        comms.connectivity(),
        comms.shutdown_signal(),
    )
    .with_config(DhtConfig {
        dedup_max_allowed_occurrences: 1,
        num_neighbouring_nodes: NUM_NEIGHBOURING_NODES,
        num_random_nodes: NUM_RANDOM_NODES,
        propagation_factor: PROPAGATION_FACTOR,
        discovery_request_timeout: Duration::from_secs(15),
        saf_auto_request: true,
        auto_join: true,
        ..DhtConfig::default_local_test()
    })
    .finish()
    .await
    .unwrap();

    let dht_outbound_layer = dht.outbound_middleware_layer();

    let comms = comms
        .with_messaging_pipeline(
            pipeline::Builder::new()
                .outbound_buffer_size(10)
                .with_outbound_pipeline(outbound_rx, |sink| {
                    ServiceBuilder::new().layer(dht_outbound_layer).service(sink)
                })
                .max_concurrent_inbound_tasks(10)
                .with_inbound_pipeline(
                    ServiceBuilder::new()
                        .layer(dht.inbound_middleware_layer())
                        .service(SinkService::new(inbound_tx)),
                )
                .finish(),
        )
        .spawn()
        .await
        .unwrap();

    (comms, dht)
}

async fn take_a_break() {
    banner!("Taking a break for a few seconds to let things settle...");
    time::delay_for(Duration::from_millis(NUM_NODES as u64 * 100)).await;
}
