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

use super::error::HorizonSyncError;
use crate::{
    base_node::{
        state_machine_service::{
            states::{helpers, helpers::exclude_sync_peer},
            BaseNodeStateMachine,
        },
        sync::{SyncPeer, rpc, SyncPeers},
    },
    chain_storage::{async_db::AsyncBlockchainDb, BlockchainBackend, MmrTree},
    iterators::NonOverlappingIntegerPairIter,
    transactions::{
        transaction::{TransactionKernel, TransactionOutput},
        types::HashOutput,
    },
};
use croaring::Bitmap;
use log::*;
use tari_common_types::chain_metadata::ChainMetadata;
use tari_crypto::tari_utilities::Hashable;
use tari_comms::PeerConnection;
use crate::chain_storage::{ChainStorageError, include_legacy_deleted_hash};
use crate::proto::generated::base_node::SyncKernelsRequest;
use futures::StreamExt;
use std::convert::TryInto;
use tari_mmr::MerkleMountainRange;
use crate::transactions::types::HashDigest;
use crate::validation::ValidationError;
use crate::blocks::BlockValidationError;
use tari_crypto::tari_utilities::hex::Hex;

const LOG_TARGET: &str = "c::bn::state_machine_service::states::horizon_state_sync";

pub struct HorizonStateSynchronization<'a, B: BlockchainBackend> {
    shared: &'a mut BaseNodeStateMachine<B>,
    sync_peer: PeerConnection,
    local_metadata: &'a ChainMetadata,
    horizon_sync_height: u64,
}

impl<'a, B: BlockchainBackend + 'static> HorizonStateSynchronization<'a, B> {
    pub fn new(
        shared: &'a mut BaseNodeStateMachine<B>,
        sync_peer: PeerConnection,
        local_metadata: &'a ChainMetadata,
        horizon_sync_height: u64,
    ) -> Self
    {
        Self {
            shared,
            sync_peer,
            local_metadata,
            horizon_sync_height,
        }
    }

    pub async fn synchronize(&mut self) -> Result<(), HorizonSyncError> {
        debug!(target: LOG_TARGET, "Preparing database for horizon sync");
        self.prepare_for_sync().await?;

        match self.begin_sync().await {
            Ok(_) => match self.finalize_horizon_sync().await {
                Ok(_) => Ok(()),
                Err(err) if err.is_recoverable() => Err(err),
                Err(err) => {
                    warn!(target: LOG_TARGET, "Error during sync:{}", err);
                    self.rollback().await?;
                    Err(err)
                },
            },
            Err(err) if err.is_recoverable() => Err(err),
            Err(err) => {
                warn!(target: LOG_TARGET, "Error during sync:{}", err);
                self.rollback().await?;
                Err(err)
            },
        }
    }

    async fn begin_sync(&mut self) -> Result<(), HorizonSyncError> {
        debug!(target: LOG_TARGET, "Synchronizing kernels");
        self.synchronize_kernels().await?;
        debug!(target: LOG_TARGET, "Check the deletion state of current UTXOs");
        self.check_state_of_current_utxos().await?;
        debug!(target: LOG_TARGET, "Synchronizing UTXOs and RangeProofs");
        self.synchronize_utxos_and_rangeproofs().await?;

        Ok(())
    }

    // Synchronize kernels upto the horizon sync height from remote sync peers.
    async fn synchronize_kernels(&mut self) -> Result<(), HorizonSyncError> {
        let config = self.shared.config.horizon_sync_config;
        let local_num_kernels = self
            .db()
            .fetch_mmr_size(MmrTree::Kernel)
            .await?;

        let header= self.db().fetch_header(self.horizon_sync_height).await?.ok_or_else(||
            ChainStorageError::ValueNotFound {
                entity: "Header".to_string(),
                field: "height".to_string(),
                value: self.horizon_sync_height.to_string()
            })?;

        let remote_num_kernels = header.kernel_mmr_size;

        if local_num_kernels >= remote_num_kernels {
            debug!(target: LOG_TARGET, "Local kernel set already synchronized");
            return Ok(());
        }

        debug!(
            target: LOG_TARGET,
            "Requesting kernels from {} to {} ({} remaining)",
            local_num_kernels,
            remote_num_kernels,
            remote_num_kernels - local_num_kernels,
        );

            self.sync_kernel_nodes(local_num_kernels, remote_num_kernels).await?;

                // let (kernel_hashes, _, sync_peer1) = helpers::request_mmr_nodes(
                //     LOG_TARGET,
                //     self.shared,
                //     self.sync_peers,
                //     MmrTree::Kernel,
                //     pos,
                //     count,
                //     self.horizon_sync_height,
                //     config.max_sync_request_retry_attempts,
                // )
                // .await?;
                // let (kernels, sync_peer2) = helpers::request_kernels(
                //     LOG_TARGET,
                //     self.shared,
                //     self.sync_peers,
                //     kernel_hashes.clone(),
                //     config.max_sync_request_retry_attempts,
                // )
                // .await?;
                //
                // match self.validate_kernel_response(&kernel_hashes, &kernels) {
                //     Ok(_) => {
                //         let num_kernels = kernels.len();
                //         self.db().horizon_sync_insert_kernels(kernels).await?;
                //         trace!(
                //             target: LOG_TARGET,
                //             "{} kernels successfully added to database ({} remaining)",
                //             num_kernels,
                //             remote_num_kernels - pos,
                //         );
                //         break;
                //     },
                //     Err(err @ HorizonSyncError::EmptyResponse { .. }) |
                //     Err(err @ HorizonSyncError::IncorrectResponse { .. }) |
                //     Err(err @ HorizonSyncError::InvalidKernelSignature(_)) => {
                //         warn!(target: LOG_TARGET, "{}", err);
                //         // TODO: Fetching mmr nodes and kernels should both be attempted for the same peer
                //         if sync_peer1 == sync_peer2 {
                //             debug!(
                //                 target: LOG_TARGET,
                //                 "Banning peer {} from local node, because they supplied invalid kernels", sync_peer
                //             );
                //             self.ban_sync_peer(&sync_peer, "Peer supplied invalid kernels".to_string())
                //                 .await?;
                //         }
                //     },
                //     Err(e) => return Err(e),
                // debug!(
                //     target: LOG_TARGET,
                //     "Retrying kernel sync. Attempt {} of {}", attempt, config.max_sync_request_retry_attempts
                // );
                // if attempt == num_sync_peers {
                //     return Err(HorizonSyncError::MaxSyncAttemptsReached);
                // }
            // }


        // self.validate_mmr_root(MmrTree::Kernel).await?;
        //
        // Ok(())
        unimplemented!()
    }

    async fn sync_kernel_nodes(&mut self, start: u64, end: u64) -> Result<(), HorizonSyncError> {
        let peer = self.sync_peer.peer_node_id().clone();
        let mut client = self.sync_peer.connect_rpc::<rpc::BaseNodeSyncRpcClient>().await?;
        let latency = client.get_last_request_latency().await?;
        debug!(
            target: LOG_TARGET,
            "Initiating header sync with peer `{}` (latency = {}ms)",
            self.sync_peer.peer_node_id(),
            latency.unwrap_or_default().as_millis()
        );

        let req = SyncKernelsRequest {
            start,
            end
        };
        let mut  kernel_stream = client.sync_kernels(req).await?;

        let mut current_header = self.shared.db.fetch_header_containing_kernel_mmr(start + 1).await?;
        debug!(target: LOG_TARGET, "Found current header in progress for kernels at mmr pos: {} height:{}", start, current_header.height());
        // TODO: Allow for partial block kernels to be downloaded (maybe)
        let mut kernels = vec![];
        // let block = self.shared.db.fetch_block(current_header.height()).await?;
        // let (_, _, mut kernels) = block.block.body.dissolve();
        // debug!(target: LOG_TARGET, "{} of {} kernels have already been downloaded for this header", kernels.len(), current_header.header.kernel_mmr_size);
        let mut txn =  self.shared.db.write_transaction();
        let mut mmr_position = start;
        while let Some(kernel) = kernel_stream.next().await {
           let kernel : TransactionKernel = kernel?.try_into().map_err(HorizonSyncError::ConversionError)?;
            debug!(target: LOG_TARGET, "Kernel received from sync peer: {}", kernel);
           kernels.push(kernel.clone());
           txn.insert_kernel_via_horizon_sync(kernel, current_header.hash().clone(), mmr_position as u32);
            // TODO: validate kernel
            if mmr_position == current_header.header.kernel_mmr_size - 1 {

                // Validate root
                let block_data =self.shared.db.fetch_block_accumulated_data(current_header.header.prev_hash.clone()).await?;
                let kernel_pruned_set = block_data.dissolve().0;
                debug!(target: LOG_TARGET, "Kernel: {:?}", kernel_pruned_set);
                let mut kernel_mmr = MerkleMountainRange::<HashDigest, _>::new(kernel_pruned_set);

                for kernel in kernels.drain(..) {
                    kernel_mmr.push(kernel.hash())?;
                }

                debug!(target: LOG_TARGET, "Kernel: {:?}", kernel_mmr.get_pruned_hash_set()?);
                let mmr_root = include_legacy_deleted_hash(kernel_mmr.get_merkle_root()?);
                if mmr_root != current_header.header.kernel_mr {
                    debug!(target: LOG_TARGET, "MMR did not match for kernels, {} != {}", mmr_root.to_hex(), current_header.header.kernel_mr.to_hex());
                   return Err(HorizonSyncError::InvalidMmrRoot(MmrTree::Kernel));
                }

                txn.update_pruned_hash_set(MmrTree::Kernel, current_header.hash().clone(), kernel_mmr.get_pruned_hash_set()?);
                txn.commit().await?;
                current_header = self.shared.db.fetch_chain_header(current_header.height() + 1).await?;
            }
            mmr_position+=1;
        }

        unimplemented!()
    }

    async fn validate_mmr_root(&self, _tree: MmrTree) -> Result<(), HorizonSyncError> {
        unimplemented!()
        // debug!(target: LOG_TARGET, "Validating {} MMR root", tree);
        // if async_db::validate_merkle_root(self.db(), MmrTree::Kernel, self.horizon_sync_height).await? {
        //     debug!(
        //         target: LOG_TARGET,
        //         "{} MMR root is VALID at height {}", tree, self.horizon_sync_height
        //     );
        //     Ok(())
        // } else {
        //     warn!(
        //         target: LOG_TARGET,
        //         "{} MMR root is INVALID at height {}", tree, self.horizon_sync_height
        //     );
        //     Err(HorizonSyncError::InvalidMmrRoot(tree))
        // }
    }

    async fn ban_sync_peer(&mut self, sync_peer: &SyncPeer, reason: String) -> Result<(), HorizonSyncError> {
        unimplemented!()
        // helpers::ban_sync_peer(
        //     LOG_TARGET,
        //     &mut self.shared.connectivity,
        //     self.sync_peers,
        //     sync_peer,
        //     self.shared.config.sync_peer_config.short_term_peer_ban_duration,
        //     reason,
        // )
        // .await?;
        // Ok(())
    }

    // Checks if any existing UTXOs in the local database have been spent according to the remote state
    async fn check_state_of_current_utxos(&mut self) -> Result<(), HorizonSyncError> {
        unimplemented!()
        // let config = self.shared.config.horizon_sync_config;
        // let local_tip_height = self.local_metadata.height_of_longest_chain();
        // let local_num_utxo_nodes = self.db().fetch_mmr_node_count(MmrTree::Utxo, local_tip_height).await?;
        //
        // debug!(
        //     target: LOG_TARGET,
        //     "Checking current utxo state between {} and {}", 0, local_num_utxo_nodes
        // );
        //
        // let chunks = self.chunked_count_iter(0, local_num_utxo_nodes, config.max_utxo_mmr_node_request_size);
        // for (pos, count) in chunks {
        //     let num_sync_peers = self.sync_peers.len();
        //     for attempt in 1..=num_sync_peers {
        //         let (remote_utxo_hashes, remote_utxo_deleted, sync_peer) = helpers::request_mmr_nodes(
        //             LOG_TARGET,
        //             self.shared,
        //             self.sync_peers,
        //             MmrTree::Utxo,
        //             pos,
        //             count,
        //             self.horizon_sync_height,
        //             config.max_sync_request_retry_attempts,
        //         )
        //         .await?;
        //         let (local_utxo_hashes, local_utxo_bitmap_bytes) = self
        //             .shared
        //             .local_node_interface
        //             .fetch_mmr_nodes(MmrTree::Utxo, pos, count, self.horizon_sync_height)
        //             .await?;
        //         let local_utxo_deleted = Bitmap::deserialize(&local_utxo_bitmap_bytes);
        //
        //         match self.validate_utxo_hashes_response(&remote_utxo_hashes, &local_utxo_hashes) {
        //             Ok(_) => {
        //                 let num_hashes = local_utxo_hashes.len();
        //                 let spent_utxos = local_utxo_hashes
        //                     .into_iter()
        //                     .enumerate()
        //                     .filter_map(|(index, hash)| {
        //                         let deleted_index = pos + index as u32;
        //                         let local_deleted = local_utxo_deleted.contains(deleted_index);
        //                         let remote_deleted = remote_utxo_deleted.contains(deleted_index);
        //                         if remote_deleted && !local_deleted {
        //                             Some(hash)
        //                         } else {
        //                             None
        //                         }
        //                     })
        //                     .collect::<Vec<_>>();
        //
        //                 let num_deleted = spent_utxos.len();
        //                 self.db().horizon_sync_spend_utxos(spent_utxos).await?;
        //
        //                 debug!(
        //                     target: LOG_TARGET,
        //                     "Checked {} existing UTXO(s). Marked {} UTXO(s) as spent.", num_hashes, num_deleted
        //                 );
        //
        //                 break;
        //             },
        //             Err(err @ HorizonSyncError::IncorrectResponse) => {
        //                 warn!(
        //                     target: LOG_TARGET,
        //                     "Invalid UTXO hashes received from peer `{}`: {}", sync_peer, err
        //                 );
        //                 // Exclude the peer (without banning) as they could be on the wrong chain
        //                 exclude_sync_peer(LOG_TARGET, self.sync_peers, &sync_peer)?;
        //             },
        //             Err(e) => return Err(e),
        //         };
        //         debug!(target: LOG_TARGET, "Retrying UTXO state check. Attempt {}", attempt);
        //         if attempt == num_sync_peers {
        //             return Err(HorizonSyncError::MaxSyncAttemptsReached);
        //         }
        //     }
        // }
        //
        // Ok(())
    }

    // Synchronize UTXO MMR Nodes, RangeProof MMR Nodes and the UTXO set upto the horizon sync height from
    // remote sync peers.
    async fn synchronize_utxos_and_rangeproofs(&mut self) -> Result<(), HorizonSyncError> {
        unimplemented!()
        // let config = self.shared.config.horizon_sync_config;
        // let local_num_utxo_nodes = self
        //     .db()
        //     .fetch_mmr_node_count(MmrTree::Utxo, self.horizon_sync_height)
        //     .await?;
        // let (remote_num_utxo_nodes, _sync_peer) = helpers::request_mmr_node_count(
        //     LOG_TARGET,
        //     self.shared,
        //     self.sync_peers,
        //     MmrTree::Utxo,
        //     self.horizon_sync_height,
        //     config.max_sync_request_retry_attempts,
        // )
        // .await?;
        //
        // if local_num_utxo_nodes >= remote_num_utxo_nodes {
        //     debug!(target: LOG_TARGET, "UTXOs and range proofs are already synchronized.");
        //     return Ok(());
        // }
        //
        // debug!(
        //     target: LOG_TARGET,
        //     "Synchronizing {} UTXO MMR nodes from {} to {}",
        //     remote_num_utxo_nodes - local_num_utxo_nodes,
        //     local_num_utxo_nodes,
        //     remote_num_utxo_nodes
        // );
        //
        // let chunks = self.chunked_count_iter(
        //     local_num_utxo_nodes,
        //     remote_num_utxo_nodes,
        //     config.max_utxo_mmr_node_request_size,
        // );
        // for (pos, count) in chunks {
        //     let num_sync_peers = self.sync_peers.len();
        //     for attempt in 1..=num_sync_peers {
        //         let (utxo_hashes, utxo_bitmap, sync_peer1) = helpers::request_mmr_nodes(
        //             LOG_TARGET,
        //             self.shared,
        //             self.sync_peers,
        //             MmrTree::Utxo,
        //             pos,
        //             count,
        //             self.horizon_sync_height,
        //             config.max_sync_request_retry_attempts,
        //         )
        //         .await?;
        //         let (rp_hashes, _, sync_peer2) = helpers::request_mmr_nodes(
        //             LOG_TARGET,
        //             self.shared,
        //             self.sync_peers,
        //             MmrTree::RangeProof,
        //             pos,
        //             count,
        //             self.horizon_sync_height,
        //             config.max_sync_request_retry_attempts,
        //         )
        //         .await?;
        //
        //         // Construct the list of hashes of the UTXOs that need to be requested.
        //         let mut request_utxo_hashes = Vec::new();
        //         let mut request_rp_hashes = Vec::new();
        //         let mut is_stxos = Vec::with_capacity(utxo_hashes.len());
        //         for index in 0..utxo_hashes.len() {
        //             let deleted = utxo_bitmap.contains(pos + index as u32);
        //             is_stxos.push(deleted);
        //             if !deleted {
        //                 request_utxo_hashes.push(&utxo_hashes[index]);
        //                 request_rp_hashes.push(&rp_hashes[index]);
        //             }
        //         }
        //
        //         // Download a partial UTXO set
        //         let (utxos, sync_peer3) = helpers::request_txos(
        //             LOG_TARGET,
        //             self.shared,
        //             self.sync_peers,
        //             &request_utxo_hashes,
        //             config.max_sync_request_retry_attempts,
        //         )
        //         .await?;
        //
        //         debug!(
        //             target: LOG_TARGET,
        //             "Fetched {} UTXOs ({} were not downloaded because they are spent)",
        //             utxos.len(),
        //             is_stxos.iter().filter(|x| **x).count()
        //         );
        //
        //         let db = &self.shared.db;
        //         match self.validate_utxo_and_rangeproof_response(
        //             &utxo_hashes,
        //             &rp_hashes,
        //             &request_utxo_hashes,
        //             &request_rp_hashes,
        //             &utxos,
        //         ) {
        //             Ok(_) => {
        //                 // The order of these inserts are important to ensure the MMRs are constructed correctly
        //                 // and the roots match.
        //                 for (index, is_stxo) in is_stxos.into_iter().enumerate() {
        //                     if is_stxo {
        //                         db.insert_mmr_node(MmrTree::Utxo, utxo_hashes[index].clone(), true)
        //                             .await?;
        //                         db.insert_mmr_node(MmrTree::RangeProof, rp_hashes[index].clone(), false)
        //                             .await?;
        //                     } else {
        //                         unimplemented!();
        //                         // Inserting the UTXO will also insert the corresponding UTXO and RangeProof MMR
        //                         // Nodes.
        //                         // async_db::insert_utxo(db.clone(), utxos.remove(0)).await?;
        //                     }
        //                 }
        //
        //                 unimplemented!();
        //                 // async_db::horizon_sync_create_mmr_checkpoint(self.db(), MmrTree::Utxo).await?;
        //                 // async_db::horizon_sync_create_mmr_checkpoint(self.db(), MmrTree::RangeProof).await?;
        //                 // trace!(
        //                 //     target: LOG_TARGET,
        //                 //     "{} UTXOs with MMR nodes inserted into database",
        //                 //     utxo_hashes.len()
        //                 // );
        //
        //                 // break;
        //             },
        //             Err(err @ HorizonSyncError::EmptyResponse { .. }) |
        //             Err(err @ HorizonSyncError::IncorrectResponse { .. }) => {
        //                 warn!(
        //                     target: LOG_TARGET,
        //                     "Invalid UTXOs or MMR Nodes received from peer. {}", err
        //                 );
        //                 if (sync_peer1 == sync_peer2) && (sync_peer1 == sync_peer3) {
        //                     debug!(
        //                         target: LOG_TARGET,
        //                         "Banning peer {} from local node, because they supplied invalid UTXOs or MMR Nodes",
        //                         sync_peer1
        //                     );
        //
        //                     self.ban_sync_peer(&sync_peer1, "Peer supplied invalid UTXOs or MMR Nodes".to_string())
        //                         .await?;
        //                 }
        //             },
        //             Err(e) => return Err(e),
        //         };
        //
        //         debug!(target: LOG_TARGET, "Retrying kernel sync. Attempt {}", attempt);
        //         if attempt == num_sync_peers {
        //             return Err(HorizonSyncError::MaxSyncAttemptsReached);
        //         }
        //     }
        // }
        //
        // self.validate_mmr_root(MmrTree::Utxo).await?;
        // self.validate_mmr_root(MmrTree::RangeProof).await?;
        // Ok(())
    }

    // Finalize the horizon state synchronization by setting the chain metadata to the local tip and committing
    // the horizon state to the blockchain backend.
    async fn finalize_horizon_sync(&self) -> Result<(), HorizonSyncError> {
        debug!(target: LOG_TARGET, "Validating horizon state");
        let _validator = self.shared.sync_validators.final_state.clone();
        let _horizon_sync_height = self.horizon_sync_height;
        unimplemented!()
        // let validation_result = spawn_blocking(move || {
        //
        //     validator
        //         .validate(&horizon_sync_height)
        //         .map_err(HorizonSyncError::FinalStateValidationFailed)
        // })
        // .await?;
        //
        // match validation_result {
        //     Ok(_) => {
        //         debug!(
        //             target: LOG_TARGET,
        //             "Horizon state validation succeeded! Committing horizon state."
        //         );
        //         async_db::horizon_sync_commit(self.db()).await?;
        //         Ok(())
        //     },
        //     Err(err) => {
        //         debug!(target: LOG_TARGET, "Horizon state validation failed!");
        //         Err(err)
        //     },
        // }
    }

    async fn rollback(&self) -> Result<(), HorizonSyncError> {
        error!(
            target: LOG_TARGET,
            "Horizon state sync has failed. Rolling the database back to the last consistent state."
        );

        self.db().horizon_sync_rollback().await?;
        Ok(())
    }

    // Validate the received UTXO set and, UTXO and RangeProofs MMR nodes.
    fn validate_utxo_and_rangeproof_response(
        &self,
        utxo_hashes: &[HashOutput],
        rp_hashes: &[HashOutput],
        request_utxo_hashes: &[&HashOutput],
        request_rp_hashes: &[&HashOutput],
        utxos: &[TransactionOutput],
    ) -> Result<(), HorizonSyncError>
    {
        // Check if the same number of utxo and rp MMR nodes returned
        if utxo_hashes.len() != rp_hashes.len() {
            return Err(HorizonSyncError::IncorrectResponse);
        }
        // Check that the correct number of utxos returned
        if request_utxo_hashes.len() != utxos.len() {
            return Err(HorizonSyncError::IncorrectResponse);
        }

        // Check that utxo set is the requested utxos
        if (0..request_utxo_hashes.len()).any(|i| &utxos[i].hash() != request_utxo_hashes[i]) {
            return Err(HorizonSyncError::IncorrectResponse);
        }

        // Check that utxo set matches the provided RangeProof MMR Nodes
        if (0..request_rp_hashes.len()).any(|i| &utxos[i].proof.hash() != request_rp_hashes[i]) {
            return Err(HorizonSyncError::IncorrectResponse);
        }

        Ok(())
    }

    // Validate the received UTXO set and, UTXO and RangeProofs MMR nodes.
    fn validate_utxo_hashes_response(
        &self,
        local_utxo_hashes: &[HashOutput],
        remote_utxo_hashes: &[HashOutput],
    ) -> Result<(), HorizonSyncError>
    {
        // Check that the correct number of utxo hashes returned
        if local_utxo_hashes.len() != remote_utxo_hashes.len() {
            return Err(HorizonSyncError::IncorrectResponse);
        }

        // Check that the received utxo set is the same as local
        if (0..local_utxo_hashes.len()).any(|i| local_utxo_hashes[i] != remote_utxo_hashes[i]) {
            return Err(HorizonSyncError::IncorrectResponse);
        }

        Ok(())
    }

    // Check the received set of kernels.
    fn validate_kernel_response(
        &self,
        kernel_hashes: &[HashOutput],
        kernels: &[TransactionKernel],
    ) -> Result<(), HorizonSyncError>
    {
        if kernels.is_empty() {
            return Err(HorizonSyncError::EmptyResponse);
        }
        // Check if the correct number of kernels returned
        if kernel_hashes.len() != kernels.len() {
            return Err(HorizonSyncError::IncorrectResponse);
        }

        // Check that kernel set is the requested kernels
        if kernel_hashes
            .iter()
            .enumerate()
            .any(|(i, kernel_hash)| &kernels[i].hash() != kernel_hash)
        {
            return Err(HorizonSyncError::IncorrectResponse);
        }

        for k in kernels {
            k.verify_signature().map_err(HorizonSyncError::InvalidKernelSignature)?;
        }

        Ok(())
    }

    fn chunked_count_iter(&self, start: u64, end: u64, chunk_size: usize) -> impl Iterator<Item = (u64, u64)> {
        NonOverlappingIntegerPairIter::new(start, end, chunk_size)
                    // Convert (start, end) into (start, count)
                    .map(|(pos, end)| (pos, end - pos + 1))
    }

    async fn prepare_for_sync(&mut self) -> Result<(), HorizonSyncError> {
        self.db().horizon_sync_begin().await?;
        Ok(())
    }

    #[inline]
    fn db(&self) -> &AsyncBlockchainDb<B> {
        &self.shared.db
    }
}
