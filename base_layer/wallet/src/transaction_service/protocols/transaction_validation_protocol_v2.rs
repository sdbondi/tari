// Copyright 2021. The Tari Project
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

use crate::transaction_service::{
    config::TransactionServiceConfig,
    error::{TransactionServiceError, TransactionServiceProtocolError, TransactionServiceProtocolWrapper},
    storage::{
        database::{TransactionBackend, TransactionDatabase},
        models::CompletedTransaction,
    },
};
use log::*;
use std::convert::TryInto;
use tari_common_types::types::BlockHash;
use tari_comms::{connectivity::ConnectivityRequester, types::CommsPublicKey, PeerConnection};
use tari_core::{base_node::sync::rpc::BaseNodeSyncRpcClient, blocks::BlockHeader};
use tari_crypto::tari_utilities::{hex::Hex, Hashable};

const LOG_TARGET: &str = "wallet::transaction_service::protocols::validation_protocol_v2";

pub struct TransactionValidationProtocolV2<TTransactionBackend: TransactionBackend + 'static> {
    db: TransactionDatabase<TTransactionBackend>,
    base_node_pk: CommsPublicKey,
    operation_id: u64,
    batch_size: usize,
    connectivity_requester: ConnectivityRequester,
    config: TransactionServiceConfig,
}

#[allow(unused_variables)]
impl<TTransactionBackend: TransactionBackend + 'static> TransactionValidationProtocolV2<TTransactionBackend> {
    pub fn new(
        db: TransactionDatabase<TTransactionBackend>,
        base_node_pk: CommsPublicKey,
        connectivity_requester: ConnectivityRequester,
        config: TransactionServiceConfig,
    ) -> Self {
        Self {
            operation_id: 122, // Get a real tx id
            db,
            batch_size: 10,
            base_node_pk,
            connectivity_requester,
            config,
        }
    }

    pub async fn execute(mut self) -> Result<u64, TransactionServiceProtocolError> {
        let mut base_node_connection = self
            .connectivity_requester
            .dial_peer(self.base_node_pk.clone().into())
            .await
            .for_protocol(self.operation_id)?;
        let mut client = base_node_connection
            .connect_rpc_using_builder(
                BaseNodeSyncRpcClient::builder().with_deadline(self.config.chain_monitoring_timeout),
            )
            .await
            .for_protocol(self.operation_id)?;

        self.check_for_reorgs(&mut client).await?;
        info!(
            target: LOG_TARGET,
            "Checking if transactions have been mined since last we checked"
        );
        let unmined_transactions = self
            .db
            .fetch_unmined_transactions()
            .await
            .for_protocol(self.operation_id)?;
        for batch in unmined_transactions.chunks(self.batch_size) {
            info!(
                target: LOG_TARGET,
                "Asking base node for location of {} transactions by excess",
                batch.len()
            );
            let (mined, unmined) = self.query_base_node_for_transactions(batch).await?;
            info!(
                target: LOG_TARGET,
                "Base node returned {} as mined and {} as unmined",
                mined.len(),
                unmined.len()
            );
            for tx in &mined {
                info!(target: LOG_TARGET, "Updating transaction {} as mined", tx.tx_id);
                let mined_in_block = tx.mined_in_block.as_ref().unwrap(); // TODO: sort out this wrap
                let mined_height = tx.mined_height.unwrap();
                self.update_transaction_as_mined(tx, mined_in_block, mined_height)
                    .await?;
            }
            for tx in &unmined_transactions {
                info!(target: LOG_TARGET, "Updated transaction {} as unmined", tx.tx_id);
                self.update_transaction_as_unmined(&tx).await?;
            }
        }
        Ok(self.operation_id)
    }

    async fn check_for_reorgs(
        &mut self,
        client: &mut BaseNodeSyncRpcClient,
    ) -> Result<(), TransactionServiceProtocolError> {
        info!(
            target: LOG_TARGET,
            "Checking last mined transactions to see if the base node has re-orged"
        );
        loop {
            if let Some(last_mined_transaction) = self
                .db
                .get_last_mined_transaction()
                .await
                .for_protocol(self.operation_id)
                .unwrap()
            {
                let mined_height = last_mined_transaction.mined_height.unwrap(); // TODO: fix unwrap
                let mined_in_block_hash = last_mined_transaction.mined_in_block.clone().unwrap(); // TODO: fix unwrap.
                let block_at_height = self.get_base_node_block_at_height(mined_height, client).await?;
                if block_at_height != mined_in_block_hash {
                    // Chain has reorged since we last
                    warn!(
                        target: LOG_TARGET,
                        "The block that transaction (excess:{}) was in has been reorged out, will try to find this \
                         transaction again, but these funds have potentially been re-orged out of the chain",
                        last_mined_transaction
                            .transaction
                            .body
                            .kernels()
                            .first()
                            .map(|k| k.excess.to_hex())
                            .unwrap()
                    );
                    self.update_transaction_as_unmined(&last_mined_transaction).await?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "Last mined transaction is still in the block chain according to base node."
                    );
                    break;
                }
            } else {
                // No more transactions
                break;
            }
        }
        Ok(())
    }

    async fn query_base_node_for_transactions(
        &self,
        batch: &[CompletedTransaction],
    ) -> Result<(Vec<CompletedTransaction>, Vec<CompletedTransaction>), TransactionServiceProtocolError> {
        unimplemented!()
    }

    async fn get_base_node_block_at_height(
        &mut self,
        height: u64,
        client: &mut BaseNodeSyncRpcClient,
    ) -> Result<BlockHash, TransactionServiceProtocolError> {
        let result = client
            .get_header_by_height(height)
            .await
            .for_protocol(self.operation_id)?;

        let block_header: BlockHeader = result.try_into().map_err(|s| {
            TransactionServiceProtocolError::new(
                self.operation_id,
                TransactionServiceError::InvalidMessageError(format!("Could not convert block header: {}", s)),
            )
        })?;
        Ok(block_header.hash())
    }

    async fn update_transaction_as_mined(
        &self,
        tx: &CompletedTransaction,
        mined_in_block: &BlockHash,
        mined_height: u64,
    ) -> Result<(), TransactionServiceProtocolError> {
        unimplemented!()
    }

    async fn update_transaction_as_unmined(
        &self,
        tx: &CompletedTransaction,
    ) -> Result<(), TransactionServiceProtocolError> {
        unimplemented!()
    }
}
