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
        sync::{rpc, SyncPeer, SyncPeers},
    },
    blocks::BlockValidationError,
    chain_storage::{
        async_db::AsyncBlockchainDb,
        include_legacy_deleted_hash,
        BlockchainBackend,
        ChainStorageError,
        MetadataKey,
        MetadataValue,
        MmrTree,
    },
    iterators::NonOverlappingIntegerPairIter,
    proto::generated::base_node::{SyncKernelsRequest, SyncUtxosRequest, SyncUtxosResponse},
    transactions::{
        transaction::{TransactionKernel, TransactionOutput},
        types::{HashDigest, HashOutput},
    },
    validation::ValidationError,
};
use croaring::Bitmap;
use futures::StreamExt;
use log::*;
use std::convert::TryInto;
use tari_common_types::chain_metadata::ChainMetadata;
use tari_comms::PeerConnection;
use tari_crypto::tari_utilities::{hex::Hex, Hashable};
use tari_mmr::{MerkleMountainRange, MutableMmr};

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
        debug!(target: LOG_TARGET, "Synchronizing outputs");
        self.synchronize_outputs().await?;
        Ok(())
    }

    async fn synchronize_kernels(&mut self) -> Result<(), HorizonSyncError> {
        let local_num_kernels = self.db().fetch_mmr_size(MmrTree::Kernel).await?;

        let header = self.db().fetch_header(self.horizon_sync_height).await?.ok_or_else(|| {
            ChainStorageError::ValueNotFound {
                entity: "Header".to_string(),
                field: "height".to_string(),
                value: self.horizon_sync_height.to_string(),
            }
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

        self.sync_kernel_nodes(local_num_kernels, remote_num_kernels).await
    }

    async fn sync_kernel_nodes(&mut self, start: u64, end: u64) -> Result<(), HorizonSyncError> {
        let peer = self.sync_peer.peer_node_id().clone();
        let mut client = self.sync_peer.connect_rpc::<rpc::BaseNodeSyncRpcClient>().await?;
        let latency = client.get_last_request_latency().await?;
        debug!(
            target: LOG_TARGET,
            "Initiating kernel sync with peer `{}` (latency = {}ms)",
            self.sync_peer.peer_node_id(),
            latency.unwrap_or_default().as_millis()
        );

        let req = SyncKernelsRequest { start, end };
        let mut kernel_stream = client.sync_kernels(req).await?;

        let mut current_header = self.shared.db.fetch_header_containing_kernel_mmr(start + 1).await?;
        debug!(
            target: LOG_TARGET,
            "Found current header in progress for kernels at mmr pos: {} height:{}",
            start,
            current_header.height()
        );
        // TODO: Allow for partial block kernels to be downloaded (maybe)
        let mut kernels = vec![];
        // let block = self.shared.db.fetch_block(current_header.height()).await?;
        // let (_, _, mut kernels) = block.block.body.dissolve();
        // debug!(target: LOG_TARGET, "{} of {} kernels have already been downloaded for this header", kernels.len(),
        // current_header.header.kernel_mmr_size);
        let mut txn = self.shared.db.write_transaction();
        let mut mmr_position = start;
        while let Some(kernel) = kernel_stream.next().await {
            let kernel: TransactionKernel = kernel?.try_into().map_err(HorizonSyncError::ConversionError)?;
            debug!(target: LOG_TARGET, "Kernel received from sync peer: {}", kernel);
            kernels.push(kernel.clone());
            txn.insert_kernel_via_horizon_sync(kernel, current_header.hash().clone(), mmr_position as u32);
            // TODO: validate kernel
            if mmr_position == current_header.header.kernel_mmr_size - 1 {
                // Validate root
                let block_data = self
                    .shared
                    .db
                    .fetch_block_accumulated_data(current_header.header.prev_hash.clone())
                    .await?;
                let kernel_pruned_set = block_data.dissolve().0;
                debug!(target: LOG_TARGET, "Kernel: {:?}", kernel_pruned_set);
                let mut kernel_mmr = MerkleMountainRange::<HashDigest, _>::new(kernel_pruned_set);

                for kernel in kernels.drain(..) {
                    kernel_mmr.push(kernel.hash())?;
                }

                debug!(target: LOG_TARGET, "Kernel: {:?}", kernel_mmr.get_pruned_hash_set()?);
                let mmr_root = include_legacy_deleted_hash(kernel_mmr.get_merkle_root()?);
                if mmr_root != current_header.header.kernel_mr {
                    debug!(
                        target: LOG_TARGET,
                        "MMR did not match for kernels, {} != {}",
                        mmr_root.to_hex(),
                        current_header.header.kernel_mr.to_hex()
                    );
                    return Err(HorizonSyncError::InvalidMmrRoot(MmrTree::Kernel));
                }

                txn.update_pruned_hash_set(
                    MmrTree::Kernel,
                    current_header.hash().clone(),
                    kernel_mmr.get_pruned_hash_set()?,
                );
                txn.commit().await?;
                if mmr_position < end - 1 {
                    current_header = self.shared.db.fetch_chain_header(current_header.height() + 1).await?;
                }
            }
            mmr_position += 1;
        }
        // TODO: Total kernel sum in horizon block
        Ok(())
    }

    async fn synchronize_outputs(&mut self) -> Result<(), HorizonSyncError> {
        let local_num_outputs = self.db().fetch_mmr_size(MmrTree::Utxo).await?;

        let header = self.db().fetch_header(self.horizon_sync_height).await?.ok_or_else(|| {
            ChainStorageError::ValueNotFound {
                entity: "Header".to_string(),
                field: "height".to_string(),
                value: self.horizon_sync_height.to_string(),
            }
        })?;

        let remote_num_outputs = header.output_mmr_size;

        if local_num_outputs >= remote_num_outputs {
            debug!(target: LOG_TARGET, "Local output set already synchronized");
            return Ok(());
        }

        debug!(
            target: LOG_TARGET,
            "Requesting outputs from {} to {} ({} remaining)",
            local_num_outputs,
            remote_num_outputs,
            remote_num_outputs - local_num_outputs,
        );

        self.sync_output_nodes(local_num_outputs, remote_num_outputs, header.hash())
            .await
    }

    async fn sync_output_nodes(&mut self, start: u64, end: u64, end_hash: HashOutput) -> Result<(), HorizonSyncError> {
        let peer = self.sync_peer.peer_node_id().clone();
        let mut client = self.sync_peer.connect_rpc::<rpc::BaseNodeSyncRpcClient>().await?;
        let latency = client.get_last_request_latency().await?;
        debug!(
            target: LOG_TARGET,
            "Initiating output sync with peer `{}` (latency = {}ms)",
            self.sync_peer.peer_node_id(),
            latency.unwrap_or_default().as_millis()
        );

        let req = SyncUtxosRequest {
            start,
            end_header_hash: end_hash,
        };
        let mut output_stream = client.sync_utxos(req).await?;

        let mut current_header = self.shared.db.fetch_header_containing_utxo_mmr(start + 1).await?;
        debug!(
            target: LOG_TARGET,
            "Found current header in progress for utxos at mmr pos: {} height:{}",
            start,
            current_header.height()
        );
        // TODO: Allow for partial block kernels to be downloaded (maybe)
        let mut output_hashes = vec![];
        let mut rp_hashes = vec![];
        // let block = self.shared.db.fetch_block(current_header.height()).await?;
        // let (_, _, mut kernels) = block.block.body.dissolve();
        // debug!(target: LOG_TARGET, "{} of {} kernels have already been downloaded for this header", kernels.len(),
        // current_header.header.kernel_mmr_size);
        let mut txn = self.shared.db.write_transaction();
        let mut mmr_position = start;
        while let Some(response) = output_stream.next().await {
            let res: SyncUtxosResponse = response?;
            debug!(
                target: LOG_TARGET,
                "UTXOs response received from sync peer: ({} outputs, {} deleted bitmaps)",
                res.utxos.len(),
                res.deleted_bitmaps.len()
            );
            let (utxos, mut deleted_bitmaps) = (res.utxos, res.deleted_bitmaps.into_iter());
            for utxo in utxos {
                // kernels.push(kernel.clone());

                if let Some(output) = utxo.output {
                    let output: TransactionOutput = output.try_into().map_err(HorizonSyncError::ConversionError)?;
                    output_hashes.push(output.hash());
                    rp_hashes.push(output.proof().hash());
                    txn.insert_output_via_horizon_sync(output, current_header.hash().clone(), mmr_position as u32);
                } else {
                    output_hashes.push(utxo.hash.clone());
                    rp_hashes.push(utxo.rangeproof_hash.clone());
                    txn.insert_pruned_output_via_horizon_sync(
                        utxo.hash,
                        utxo.rangeproof_hash,
                        current_header.hash().clone(),
                        mmr_position as u32,
                    );
                }

                // TODO: validate outputs
                if mmr_position == current_header.header.output_mmr_size - 1 {
                    // Validate root
                    let block_data = self
                        .shared
                        .db
                        .fetch_block_accumulated_data(current_header.header.prev_hash.clone())
                        .await?;
                    let (_, output_pruned_set, rp_pruned_set, deleted) = block_data.dissolve();
                    let mut output_mmr = MerkleMountainRange::<HashDigest, _>::new(output_pruned_set);
                    let mut proof_mmr = MerkleMountainRange::<HashDigest, _>::new(rp_pruned_set);

                    for hash in output_hashes.drain(..) {
                        output_mmr.push(hash)?;
                    }

                    for hash in rp_hashes.drain(..) {
                        proof_mmr.push(hash)?;
                    }

                    let deleted_diff = deleted_bitmaps.next();
                    if deleted_diff.is_none() {
                        return Err(HorizonSyncError::IncorrectResponse(format!(
                            "No deleted bitmap was provided for the header at height:{}",
                            current_header.height()
                        )));
                    }

                    let bitmap = Bitmap::deserialize(&deleted_diff.unwrap());
                    let deleted = deleted.or(&bitmap);
                    let pruned_output_set = output_mmr.get_pruned_hash_set()?;
                    let output_mmr = MutableMmr::<HashDigest, _>::new(pruned_output_set.clone(), deleted)?;

                    let mmr_root = output_mmr.get_merkle_root()?;
                    if mmr_root != current_header.header.output_mr {
                        debug!(
                            target: LOG_TARGET,
                            "MMR did not match for outputs, {} != {}",
                            mmr_root.to_hex(),
                            current_header.header.output_mr.to_hex()
                        );
                        return Err(HorizonSyncError::InvalidMmrRoot(MmrTree::Utxo));
                    }
                    let mmr_root = include_legacy_deleted_hash(proof_mmr.get_merkle_root()?);
                    if mmr_root != current_header.header.range_proof_mr {
                        debug!(
                            target: LOG_TARGET,
                            "MMR did not match for proofs, {} != {}",
                            mmr_root.to_hex(),
                            current_header.header.range_proof_mr.to_hex()
                        );
                        return Err(HorizonSyncError::InvalidMmrRoot(MmrTree::RangeProof));
                    }

                    txn.update_pruned_hash_set(MmrTree::Utxo, current_header.hash().clone(), pruned_output_set);
                    txn.update_pruned_hash_set(
                        MmrTree::RangeProof,
                        current_header.hash().clone(),
                        proof_mmr.get_pruned_hash_set()?,
                    );
                    txn.update_deleted(current_header.hash().clone(), output_mmr.deleted().clone());

                    txn.commit().await?;
                    if mmr_position < end - 1 {
                        current_header = self.shared.db.fetch_chain_header(current_header.height() + 1).await?;
                    }
                }
                mmr_position += 1;
            }
        }
        Ok(())
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

        // TODO: validate total sum
        let header = self.db().fetch_chain_header(self.horizon_sync_height).await?;

        self.shared
            .db
            .write_transaction()
            .set_metadata(MetadataKey::BestBlock, MetadataValue::BestBlock(header.hash().clone()))
            .set_metadata(MetadataKey::ChainHeight, MetadataValue::ChainHeight(header.height()))
            .set_metadata(
                MetadataKey::AccumulatedWork,
                MetadataValue::AccumulatedWork(header.accumulated_data.total_accumulated_difficulty),
            )
            .set_metadata(
                MetadataKey::EffectivePrunedHeight,
                MetadataValue::EffectivePrunedHeight(header.height()),
            )
            .commit().await?;

        Ok(())
        // let validator = self.shared.sync_validators.final_state.clone();
        // let horizon_sync_height = self.horizon_sync_height;

        // match validation_result {
        //     Ok(_) => {
        //         debug!(
        //             target: LOG_TARGET,
        //             "Horizon state validation succeeded! Committing horizon state."
        //         );
        //         self.db.horizon_sync_commit().await?;
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
