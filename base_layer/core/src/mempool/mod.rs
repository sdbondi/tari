// Copyright 2019. The Tari Project
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

pub mod test_utils;

mod config;
mod consts;
mod error;
#[allow(clippy::module_inception)]
mod mempool;
mod mempool_storage;
mod orphan_pool;
mod pending_pool;
mod priority;
mod reorg_pool;
mod rpc;
pub use rpc::{create_mempool_rpc_service, MempoolRpcClient, MempoolRpcServer, MempoolRpcService, MempoolService};
mod unconfirmed_pool;

// public modules
pub mod async_mempool;

// Public re-exports
pub use self::config::{MempoolConfig, MempoolServiceConfig};
pub use error::MempoolError;
pub use mempool::{Mempool, MempoolValidators};
pub use service::{MempoolServiceError, MempoolServiceInitializer, OutboundMempoolServiceInterface};

pub mod service;

mod sync_protocol;
pub use sync_protocol::MempoolSyncInitializer;

use crate::transactions::types::Signature;
use core::fmt::{Display, Error, Formatter};
use serde::{Deserialize, Serialize};
use tari_crypto::tari_utilities::hex::Hex;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatsResponse {
    pub total_txs: usize,
    pub unconfirmed_txs: usize,
    pub orphan_txs: usize,
    pub timelocked_txs: usize,
    pub published_txs: usize,
    pub total_weight: u64,
}

impl Display for StatsResponse {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), Error> {
        write!(
            fmt,
            "Mempool stats: Total transactions: {}, Unconfirmed: {}, Orphaned: {}, Time locked: {}, Published: {}, \
             Total Weight: {}",
            self.total_txs,
            self.unconfirmed_txs,
            self.orphan_txs,
            self.timelocked_txs,
            self.published_txs,
            self.total_weight
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateResponse {
    pub unconfirmed_pool: Vec<Signature>,
    pub orphan_pool: Vec<Signature>,
    pub pending_pool: Vec<Signature>,
    pub reorg_pool: Vec<Signature>,
}

impl Display for StateResponse {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), Error> {
        fmt.write_str("----------------- Mempool -----------------\n")?;
        fmt.write_str("--- Unconfirmed Pool ---\n")?;
        for excess_sig in &self.unconfirmed_pool {
            fmt.write_str(&format!("    {}\n", excess_sig.get_signature().to_hex()))?;
        }
        fmt.write_str("--- Orphan Pool ---\n")?;
        for excess_sig in &self.orphan_pool {
            fmt.write_str(&format!("    {}\n", excess_sig.get_signature().to_hex()))?;
        }
        fmt.write_str("--- Pending Pool ---\n")?;
        for excess_sig in &self.pending_pool {
            fmt.write_str(&format!("    {}\n", excess_sig.get_signature().to_hex()))?;
        }
        fmt.write_str("--- Reorg Pool ---\n")?;
        for excess_sig in &self.reorg_pool {
            fmt.write_str(&format!("    {}\n", excess_sig.get_signature().to_hex()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TxStorageResponse {
    UnconfirmedPool,
    OrphanPool,
    PendingPool,
    ReorgPool,
    NotStored,
}

impl TxStorageResponse {
    pub fn is_stored(&self) -> bool {
        match self {
            TxStorageResponse::NotStored => false,
            _ => true,
        }
    }
}

impl Display for TxStorageResponse {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), Error> {
        let storage = match self {
            TxStorageResponse::UnconfirmedPool => "Unconfirmed pool",
            TxStorageResponse::OrphanPool => "Orphan pool",
            TxStorageResponse::PendingPool => "Pending pool",
            TxStorageResponse::ReorgPool => "Reorg pool",
            TxStorageResponse::NotStored => "Not stored",
        };
        fmt.write_str(&storage)
    }
}

/// Events that can be published on state changes of the Mempool
#[derive(Debug, Clone)]
pub enum MempoolStateEvent {
    Updated,
}
