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
use crate::output_manager_service::{
    config::OutputManagerServiceConfig,
    error::{OutputManagerError, OutputManagerProtocolError, OutputManagerProtocolErrorExt},
    handle::{OutputManagerEvent, OutputManagerEventSender},
    storage::{
        database::{OutputManagerBackend, OutputManagerDatabase},
        models::DbUnblindedOutput,
    },
    TxoValidationType,
};
use log::*;
use std::{collections::HashMap, convert::TryInto, sync::Arc};
use tari_common_types::types::BlockHash;
use tari_comms::{
    connectivity::ConnectivityRequester,
    protocol::rpc::{RpcError::RequestFailed, RpcStatusCode::NotFound},
    types::CommsPublicKey,
};
use tari_core::{
    base_node::{rpc::BaseNodeWalletRpcClient, sync::rpc::BaseNodeSyncRpcClient},
    blocks::BlockHeader,
    proto::base_node::UtxoQueryRequest,
};
use tari_crypto::tari_utilities::{hex::Hex, Hashable};
use tari_shutdown::ShutdownSignal;

const LOG_TARGET: &str = "wallet::output_service::txo_validation_task_v2";

pub struct TxoValidationTaskV2<TBackend: OutputManagerBackend + 'static> {
    base_node_pk: CommsPublicKey,
    db: OutputManagerDatabase<TBackend>,
    operation_id: u64,
    batch_size: usize,
    connectivity_requester: ConnectivityRequester,
    event_publisher: OutputManagerEventSender,
    validation_type: TxoValidationType,
    config: OutputManagerServiceConfig,
}

impl<TBackend> TxoValidationTaskV2<TBackend>
where TBackend: OutputManagerBackend + 'static
{
    pub fn new(
        base_node_pk: CommsPublicKey,
        operation_id: u64,
        batch_size: usize,
        db: OutputManagerDatabase<TBackend>,
        connectivity_requester: ConnectivityRequester,
        event_publisher: OutputManagerEventSender,
        validation_type: TxoValidationType,
        config: OutputManagerServiceConfig,
    ) -> Self {
        Self {
            base_node_pk,
            operation_id,
            batch_size,
            db,
            connectivity_requester,
            event_publisher,
            validation_type,
            config,
        }
    }

    pub async fn execute(mut self, _shutdown: ShutdownSignal) -> Result<u64, OutputManagerProtocolError> {
        let (mut sync_client, mut wallet_client) = self.create_base_node_clients().await?;

        info!(
            target: LOG_TARGET,
            "Starting TXO validation protocol V2 (Id: {}) for {}", self.operation_id, self.validation_type,
        );

        self.check_for_reorgs(&mut sync_client).await?;

        let unmined_outputs = self
            .db
            .fetch_unmined_outputs()
            .await
            .for_protocol(self.operation_id)
            .unwrap();

        for batch in unmined_outputs.chunks(self.batch_size) {
            info!(
                target: LOG_TARGET,
                "Asking base node for location of {} outputs by hash",
                batch.len()
            );
            let (mined, unmined) = self
                .query_base_node_for_outputs(batch, &mut wallet_client)
                .await
                .for_protocol(self.operation_id)?;
            info!(
                target: LOG_TARGET,
                "Base node returned {} as mined and {} as unmined",
                mined.len(),
                unmined.len()
            );
            for (tx, mined_height, mined_in_block, mmr_position) in &mined {
                info!(
                    target: LOG_TARGET,
                    "Updating output comm:{}: hash{} as mined",
                    tx.commitment.to_hex(),
                    tx.hash.to_hex()
                );
                self.update_output_as_mined(&tx, mined_in_block, *mined_height, *mmr_position)
                    .await?;
            }
        }

        self.publish_event(OutputManagerEvent::TxoValidationSuccess(
            self.operation_id,
            self.validation_type,
        ));
        Ok(self.operation_id)
    }

    async fn create_base_node_clients(
        &mut self,
    ) -> Result<(BaseNodeSyncRpcClient, BaseNodeWalletRpcClient), OutputManagerProtocolError> {
        let mut base_node_connection = self
            .connectivity_requester
            .dial_peer(self.base_node_pk.clone().into())
            .await
            .for_protocol(self.operation_id)?;
        let sync_client = base_node_connection
            .connect_rpc_using_builder(
                BaseNodeSyncRpcClient::builder().with_deadline(self.config.base_node_query_timeout),
            )
            .await
            .for_protocol(self.operation_id)?;
        let wallet_client = base_node_connection
            .connect_rpc_using_builder(
                BaseNodeWalletRpcClient::builder().with_deadline(self.config.base_node_query_timeout),
            )
            .await
            .for_protocol(self.operation_id)?;

        Ok((sync_client, wallet_client))
    }

    async fn check_for_reorgs(&mut self, client: &mut BaseNodeSyncRpcClient) -> Result<(), OutputManagerProtocolError> {
        info!(
            target: LOG_TARGET,
            "Checking last mined TXO to see if the base node has re-orged"
        );
        loop {
            if let Some(last_mined_output) = self
                .db
                .get_last_mined_output()
                .await
                .for_protocol(self.operation_id)
                .unwrap()
            {
                let mined_height = last_mined_output.mined_height.unwrap(); // TODO: fix unwrap
                let mined_in_block_hash = last_mined_output.mined_in_block.clone().unwrap(); // TODO: fix unwrap.
                let block_at_height = self
                    .get_base_node_block_at_height(mined_height, client)
                    .await
                    .for_protocol(self.operation_id)?;
                if block_at_height.is_none() || block_at_height.unwrap() != mined_in_block_hash {
                    // Chain has reorged since we last
                    warn!(
                        target: LOG_TARGET,
                        "The block that output (commitment) was in has been reorged out, will try to find this output \
                         again, but these funds have potentially been re-orged out of the chain",
                    );
                    // unimplemented!("todo");
                    // self.update_transaction_as_unmined(&last_mined_transaction).await?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "Last mined transaction is still in the block chain according to base node."
                    );
                    break;
                }
            } else {
                // No more outputs
                break;
            }
        }

        loop {
            if let Some(last_spent_output) = self
                .db
                .get_last_spent_output()
                .await
                .for_protocol(self.operation_id)
                .unwrap()
            {
                let mined_height = last_spent_output.marked_deleted_at_height.unwrap(); // TODO: fix unwrap
                let mined_in_block_hash = last_spent_output.marked_deleted_in_block.clone().unwrap(); // TODO: fix unwrap.
                let block_at_height = self
                    .get_base_node_block_at_height(mined_height, client)
                    .await
                    .for_protocol(self.operation_id)?;
                if block_at_height.is_none() || block_at_height.unwrap() != mined_in_block_hash {
                    // Chain has reorged since we last
                    warn!(
                        target: LOG_TARGET,
                        "The block that output (commitment) was spent in has been reorged out, will try to find this \
                         output again, but these funds have potentially been re-orged out of the chain",
                    );
                    unimplemented!("todo");
                    // self.update_transaction_as_unmined(&last_mined_transaction).await?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "Last mined transaction is still in the block chain according to base node."
                    );
                    break;
                }
            } else {
                // No more outputs
                break;
            }
        }
        Ok(())
    }

    // TODO: remove this duplicated code from transaction validation protocol

    async fn get_base_node_block_at_height(
        &mut self,
        height: u64,
        client: &mut BaseNodeSyncRpcClient,
    ) -> Result<Option<BlockHash>, OutputManagerError> {
        let result = match client.get_header_by_height(height).await {
            Ok(r) => r,
            Err(rpc_error) => {
                warn!(target: LOG_TARGET, "Error asking base node for header:{}", rpc_error);
                match &rpc_error {
                    RequestFailed(status) => {
                        if status.status_code() == NotFound {
                            return Ok(None);
                        } else {
                            return Err(rpc_error.into());
                        }
                    },
                    _ => {
                        return Err(rpc_error.into());
                    },
                }
            },
        };

        let block_header: BlockHeader = result
            .try_into()
            .map_err(|s| OutputManagerError::InvalidMessageError(format!("Could not convert block header: {}", s)))?;
        Ok(Some(block_header.hash()))
    }

    async fn query_base_node_for_outputs(
        &self,
        batch: &[DbUnblindedOutput],
        base_node_client: &mut BaseNodeWalletRpcClient,
    ) -> Result<(Vec<(DbUnblindedOutput, u64, BlockHash, u64)>, Vec<DbUnblindedOutput>), OutputManagerError> {
        let mut batch_hashes = batch.iter().map(|o| o.hash.clone()).collect();

        let batch_response = base_node_client
            .utxo_query(UtxoQueryRequest {
                output_hashes: batch_hashes,
            })
            .await?;

        let mut mined = vec![];
        let mut unmined = vec![];

        let mut returned_outputs = HashMap::new();
        for output_proto in batch_response.responses.iter() {
            returned_outputs.insert(output_proto.output_hash.clone(), output_proto);
        }

        for output in batch {
            if let Some(returned_output) = returned_outputs.get(&output.hash) {
                mined.push((
                    output.clone(),
                    returned_output.mined_height,
                    returned_output.mined_in_block.clone(),
                    returned_output.mmr_position,
                ))
            } else {
                unmined.push(output.clone());
            }
        }

        Ok((mined, unmined))
    }

    async fn update_output_as_mined(
        &self,
        tx: &DbUnblindedOutput,
        mined_in_block: &BlockHash,
        mined_height: u64,
        mmr_position: u64,
    ) -> Result<(), OutputManagerProtocolError> {
        self.db
            .set_output_mined_height(tx.hash.clone(), mined_height, mined_in_block.clone(), mmr_position)
            .await
            .for_protocol(self.operation_id)?;

        // if num_confirmations >= self.config.num_confirmations_required {
        //     self.publish_event(TransactionEvent::TransactionMined(tx.tx_id))
        // } else {
        //     self.publish_event(TransactionEvent::TransactionMinedUnconfirmed(
        //         tx.tx_id,
        //         num_confirmations,
        //     ))
        // }

        Ok(())
    }

    fn publish_event(&self, event: OutputManagerEvent) {
        match self.event_publisher.send(Arc::new(event)) {
            Err(e) => {
                debug!(
                    target: LOG_TARGET,
                    "Error sending event because there are no subscribers: {:?}", e
                );
            },
            _ => (),
        }
    }
}
