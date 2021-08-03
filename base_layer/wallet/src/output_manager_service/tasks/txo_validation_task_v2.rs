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
    error::{OutputManagerProtocolError, OutputManagerProtocolErrorExt},
    handle::OutputManagerEventSender,
    storage::database::{OutputManagerBackend, OutputManagerDatabase},
    TxoValidationType,
};
use log::*;
use tari_comms::{connectivity::ConnectivityRequester, types::CommsPublicKey};
use tari_core::base_node::sync::rpc::BaseNodeSyncRpcClient;
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
        connectivity_requester: ConnectivityRequester,
        event_publisher: OutputManagerEventSender,
        validation_type: TxoValidationType,
        config: OutputManagerServiceConfig,
    ) -> Self {
        Self {
            base_node_pk,
            operation_id,
            batch_size,
            connectivity_requester,
            event_publisher,
            validation_type,
            config,
        }
    }

    pub async fn execute(mut self, shutdown: ShutdownSignal) -> Result<(), OutputManagerProtocolError> {
        let mut base_node_client = self.create_base_node_client().await?;

        info!(
            target: LOG_TARGET,
            "Starting TXO validation protocol V2 (Id: {}) for {}", self.operation_id, self.validation_type,
        );

        self.check_for_reorgs(&mut base_node_client).await?;
        unimplemented!()
    }

    async fn create_base_node_client(&mut self) -> Result<BaseNodeSyncRpcClient, OutputManagerProtocolError> {
        let mut base_node_connection = self
            .connectivity_requester
            .dial_peer(self.base_node_pk.clone().into())
            .await
            .for_protocol(self.operation_id)?;
        let mut client = base_node_connection
            .connect_rpc_using_builder(
                BaseNodeSyncRpcClient::builder().with_deadline(self.config.base_node_query_timeout),
            )
            .await
            .for_protocol(self.operation_id)?;

        Ok(client)
    }

    async fn check_for_reorgs(&mut self, client: &mut BaseNodeSyncRpcClient) -> Result<(), OutputManagerProtocolError> {
        info!(
            target: LOG_TARGET,
            "Checking last mined TXO to see if the base node has re-orged"
        );
        loop {
            if let Some(last_mined_output) = self
                .db
                .get_last_mined_output(self.validation_type)
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
                    // self.update_transaction_as_unmined(&last_mined_transaction).await?;
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
}
