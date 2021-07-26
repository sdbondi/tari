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

use crate::{
    common::rate_limit::RateLimit,
    message::InboundMessage,
    peer_manager::NodeId,
    protocol::messaging::{MessagingEvent, MessagingProtocol},
};
use futures::{channel::mpsc, future::Either, AsyncRead, AsyncWrite, SinkExt, StreamExt};
use log::*;
use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast;

const LOG_TARGET: &str = "comms::protocol::messaging::inbound";

pub struct InboundMessaging {
    peer: NodeId,
    inbound_message_tx: mpsc::Sender<InboundMessage>,
    messaging_events_tx: broadcast::Sender<Arc<MessagingEvent>>,
    rate_limit_capacity: usize,
    rate_limit_restock_interval: Duration,
    inactivity_timeout: Option<Duration>,
}

impl InboundMessaging {
    pub fn new(
        peer: NodeId,
        inbound_message_tx: mpsc::Sender<InboundMessage>,
        messaging_events_tx: broadcast::Sender<Arc<MessagingEvent>>,
        rate_limit_capacity: usize,
        rate_limit_restock_interval: Duration,
        inactivity_timeout: Option<Duration>,
    ) -> Self {
        Self {
            peer,
            inbound_message_tx,
            messaging_events_tx,
            rate_limit_capacity,
            rate_limit_restock_interval,
            inactivity_timeout,
        }
    }

    pub async fn run<S>(mut self, socket: S)
    where S: AsyncRead + AsyncWrite + Unpin {
        let peer = &self.peer;
        debug!(
            target: LOG_TARGET,
            "Starting inbound messaging protocol for peer '{}'",
            peer.short_str()
        );

        let (mut sink, stream) = MessagingProtocol::framed(socket).split();

        if let Err(err) = sink.close().await {
            debug!(
                target: LOG_TARGET,
                "Error closing sink half for peer `{}`: {}",
                peer.short_str(),
                err
            );
        }
        let stream = stream.rate_limit(self.rate_limit_capacity, self.rate_limit_restock_interval);

        let mut stream = match self.inactivity_timeout {
            Some(timeout) => Either::Left(tokio::stream::StreamExt::timeout(stream, timeout)),
            None => Either::Right(stream.map(Ok)),
        };

        while let Some(result) = stream.next().await {
            match result {
                Ok(Ok(raw_msg)) => {
                    let inbound_msg = InboundMessage::new(peer.clone(), raw_msg.clone().freeze());
                    debug!(
                        target: LOG_TARGET,
                        "Received message {} from peer '{}' ({} bytes)",
                        inbound_msg.tag,
                        peer.short_str(),
                        raw_msg.len()
                    );

                    let event = MessagingEvent::MessageReceived(inbound_msg.source_peer.clone(), inbound_msg.tag);

                    if let Err(err) = self.inbound_message_tx.send(inbound_msg).await {
                        warn!(
                            target: LOG_TARGET,
                            "Failed to send InboundMessage for peer '{}' because '{}'",
                            peer.short_str(),
                            err
                        );

                        if err.is_disconnected() {
                            break;
                        }
                    }

                    let _ = self.messaging_events_tx.send(Arc::new(event));
                },
                Ok(Err(err)) => {
                    error!(
                        target: LOG_TARGET,
                        "Failed to receive from peer '{}' because '{}'",
                        peer.short_str(),
                        err
                    );
                    break;
                },

                Err(_) => {
                    debug!(
                        target: LOG_TARGET,
                        "Inbound messaging for peer '{}' has stopped because it was inactive for {:.0?}",
                        peer.short_str(),
                        self.inactivity_timeout
                            .expect("Inactivity timeout reached but it was not enabled"),
                    );
                    break;
                },
            }
        }

        debug!(
            target: LOG_TARGET,
            "Inbound messaging handler exited for peer `{}`",
            peer.short_str()
        );
    }
}
