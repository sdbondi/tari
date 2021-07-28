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
    error::TransactionServiceProtocolError,
    storage::database::{TransactionBackend, TransactionDatabase},
};
use tari_common_types::types::BlockHash;
use tari_core::transactions::transaction::Transaction;

pub struct TransactionValidationProtocolV2<TTransactionBackend: TransactionBackend + 'static> {
    db: TransactionDatabase<TBackend>,
    operation_id: u64,
    batch_size: usize,
}

impl<TTransactionBackend: TransactionBackend + 'static> TransactionValidationProtocolV2<TTransactionBackend> {
    pub fn new(db: TransactionDatabase<TTransactionBackend>) -> Self {
        Self {
            operation_id: 122, // Get a real tx id
            db,
            batch_size: 10,
        }
    }

    pub async fn execute(mut self) -> Result<u64, TransactionServiceProtocolError> {
        self.check_for_reorgs().await?;
        let unmined_transactions = self.fetch_unmined_transactions().await?;
        for batch in unmined_transactions.chunks(self.batch_size) {
            let (mined, unmined) = self.query_base_node_for_transactions(batch).await?;
            for (tx, mined_in_block, mined_height) in &mined {
                self.update_transaction_as_mined(tx, mined_in_block, *mined_height)
                    .await?;
            }
            for tx in &unmined_transactions {
                self.update_transaction_as_unmined(tx).await?;
            }
        }
        Ok(self.operation_id)
    }

    async fn check_for_reorgs(&self) -> Result<(), TransactionServiceProtocolError> {
        loop {
            if let Some((last_mined_transaction, mined_in_block_hash, mined_height)) =
                self.fetch_last_mined_transaction().await?
            {
                let block_at_height = self.get_base_node_block_at_height(mined_height).await?;
                if block_at_height != mined_in_block_hash {
                    // Chain has reorged since we last
                    self.update_transaction_as_unmined(&last_mined_transaction).await?;
                } else {
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
        batch: &[Transaction],
    ) -> Result<(Vec<(Transaction, BlockHash, u64)>, Vec<Transaction>), TransactionServiceProtocolError> {
        unimplemented!()
    }

    async fn fetch_unmined_transactions(&self) -> Result<Vec<Transaction>, TransactionServiceProtocolError> {
        unimplemented!()
    }

    async fn get_last_mined_transaction(
        &self,
    ) -> Result<Option<(Transaction, BlockHash, u64)>, TransactionServiceProtocolError> {
        self.db.get_last_mined_transaction().await
    }

    async fn get_base_node_block_at_height(&self, height: u64) -> Result<BlockHash, TransactionServiceProtocolError> {
        unimplemented!()
    }

    async fn update_transaction_as_mined(
        &self,
        tx: &Transaction,
        mined_in_block: &BlockHash,
        mined_height: u64,
    ) -> Result<(), TransactionServiceProtocolError> {
        unimplemented!()
    }

    async fn update_transaction_as_unmined(&self, tx: &Transaction) -> Result<(), TransactionServiceProtocolError> {
        unimplemented!()
    }
}
