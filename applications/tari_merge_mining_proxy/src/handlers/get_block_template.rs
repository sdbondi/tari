//  Copyright 2021, The Tari Project
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
    block_template_data::{BlockTemplateDataBuilder, BlockTemplateRepository},
    common::{merge_mining, proxy},
    error::MmProxyError,
    handlers::{helpers, ProxyHandler, ProxyHandlerJsonRpcPredicate, ProxyHandlerPredicate},
};
use async_trait::async_trait;
use hyper::{Body, Request, Response};
use jsonrpc::serde_json::Value;
use log::*;
use std::{
    cmp,
    convert::TryFrom,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tari_core::{
    proof_of_work::monero_rx,
    proto::core::{Block, NewBlockTemplate},
};

const LOG_TARGET: &str = "mmproxy::handlers::get_block_template";

pub struct GetBlockTemplateHandler {
    initial_sync_achieved: Arc<AtomicBool>,
    block_templates: BlockTemplateRepository,
}

impl ProxyHandlerJsonRpcPredicate for GetBlockTemplateHandler {
    const SUPPORTED_METHODS: &'static [&'static str] = &["getblocktemplate", "get_block_template"];
}

#[async_trait]
impl ProxyHandler for GetBlockTemplateHandler {
    type Error = MmProxyError;

    async fn handle(
        &self,
        request: Request<Value>,
        monero_resp: Response<Value>,
    ) -> Result<Response<Body>, Self::Error>
    {
        let (parts, mut monerod_resp) = monerod_resp.into_parts();
        debug!(
            target: LOG_TARGET,
            "handle_get_block_template: monero block #{}", monerod_resp["result"]["height"]
        );

        // If monderod returned an error, there is nothing further for us to do
        if !monerod_resp["error"].is_null() {
            return Ok(proxy::into_response(parts, &monerod_resp));
        }

        if monerod_resp["result"]["difficulty"].is_null() {
            return Err(MmProxyError::InvalidMonerodResponse(
                "Expected `get_block_template` to include `result.difficulty` but it was `null`".to_string(),
            ));
        }

        if monerod_resp["result"]["blocktemplate_blob"].is_null() {
            return Err(MmProxyError::InvalidMonerodResponse(
                "Expected `get_block_template` to include `result.blocktemplate_blob` but it was `null`".to_string(),
            ));
        }

        if monerod_resp["result"]["blockhashing_blob"].is_null() {
            return Err(MmProxyError::InvalidMonerodResponse(
                "Expected `get_block_template` to include `result.blockhashing_blob` but it was `null`".to_string(),
            ));
        }

        if monerod_resp["result"]["seed_hash"].is_null() {
            return Err(MmProxyError::InvalidMonerodResponse(
                "Expected `get_block_template` to include `result.seed_hash` but it was `null`".to_string(),
            ));
        }

        let mut grpc_client = self.connect_grpc_client().await?;

        // Add merge mining tag on blocktemplate request
        debug!(target: LOG_TARGET, "Requested new block template from Tari base node");

        let grpc::NewBlockTemplateResponse {
            miner_data,
            new_block_template,
            initial_sync_achieved,
        } = grpc_client
            .get_new_block_template(grpc::PowAlgo {
                pow_algo: grpc::pow_algo::PowAlgos::Monero.into(),
            })
            .await
            .map_err(|status| MmProxyError::GrpcRequestError {
                status,
                details: "failed to get new block template".to_string(),
            })?
            .into_inner();

        let miner_data = miner_data.ok_or_else(|| MmProxyError::GrpcResponseMissingField("miner_data"))?;
        let new_block_template =
            new_block_template.ok_or_else(|| MmProxyError::GrpcResponseMissingField("new_block_template"))?;

        let block_reward = miner_data.reward;
        let total_fees = miner_data.total_fees;
        let tari_difficulty = miner_data.target_difficulty;

        if !self.initial_sync_achieved.load(Ordering::Relaxed) {
            if !initial_sync_achieved {
                let msg = format!(
                    "Initial base node sync not achieved, current height at #{} ... (waiting = {})",
                    new_block_template.header.as_ref().map(|h| h.height).unwrap_or_default(),
                    self.config.wait_for_initial_sync_at_startup,
                );
                debug!(target: LOG_TARGET, "{}", msg);
                println!("{}", msg);
                if self.config.wait_for_initial_sync_at_startup {
                    return Err(MmProxyError::MissingDataError(" ".to_string() + &msg));
                }
            } else {
                self.initial_sync_achieved.store(true, Ordering::Relaxed);
                let msg = format!(
                    "Initial base node sync achieved at height #{}",
                    new_block_template.header.as_ref().map(|h| h.height).unwrap_or_default()
                );
                debug!(target: LOG_TARGET, "{}", msg);
                println!("{}", msg);
                println!("Listening on {}...", self.config.proxy_host_address);
            }
        }

        info!(
            target: LOG_TARGET,
            "Received new block template from Tari base node for height #{}",
            new_block_template.header.as_ref().map(|h| h.height).unwrap_or_default(),
        );

        let template_block = NewBlockTemplate::try_from(new_block_template)
            .map_err(|e| MmProxyError::MissingDataError(format!("GRPC Conversion Error: {}", e)))?;
        let tari_height = template_block.header.height;

        debug!(target: LOG_TARGET, "Trying to connect to wallet");
        let mut grpc_wallet_client = self.connect_grpc_wallet_client().await?;
        let coinbase_response = grpc_wallet_client
            .get_coinbase(GetCoinbaseRequest {
                reward: block_reward,
                fee: total_fees,
                height: tari_height,
            })
            .await
            .map_err(|status| MmProxyError::GrpcRequestError {
                status,
                details: "failed to get new block template".to_string(),
            })?;
        let coinbase_transaction = coinbase_response.into_inner().transaction;

        let coinbased_block = merge_mining::add_coinbase(coinbase_transaction, template_block)?;
        debug!(target: LOG_TARGET, "Added coinbase to new block template");
        let block = grpc_client
            .get_new_block(coinbased_block)
            .await
            .map_err(|status| MmProxyError::GrpcRequestError {
                status,
                details: "failed to get new block".to_string(),
            })?
            .into_inner();

        let mining_hash = block.merge_mining_hash;

        let tari_block = Block::try_from(
            block
                .block
                .clone()
                .ok_or_else(|| MmProxyError::MissingDataError("Tari block".to_string()))?,
        )
        .map_err(MmProxyError::MissingDataError)?;
        debug!(target: LOG_TARGET, "New block received from Tari: {}", (tari_block));

        let block_data = BlockTemplateDataBuilder::default();
        let block_data = block_data
            .tari_block(
                block
                    .block
                    .ok_or_else(|| MmProxyError::GrpcResponseMissingField("block"))?,
            )
            .tari_miner_data(miner_data);

        // Deserialize the block template blob
        let block_template_blob = &monerod_resp["result"]["blocktemplate_blob"]
            .to_string()
            .replace("\"", "");
        debug!(target: LOG_TARGET, "Deserializing Blocktemplate Blob into Monero Block",);
        let mut monero_block = merge_mining::deserialize_monero_block_from_hex(block_template_blob)?;

        debug!(target: LOG_TARGET, "Appending Merged Mining Tag",);
        // Add the Tari merge mining tag to the retrieved block template
        monero_rx::append_merge_mining_tag(&mut monero_block, &mining_hash)?;

        debug!(target: LOG_TARGET, "Creating blockhashing blob from blocktemplate blob",);
        // Must be done after the tag is inserted since it will affect the hash of the miner tx
        let blockhashing_blob = monero_rx::create_blockhashing_blob(&monero_block)?;

        debug!(target: LOG_TARGET, "blockhashing_blob:{}", blockhashing_blob);
        monerod_resp["result"]["blockhashing_blob"] = blockhashing_blob.into();

        let blocktemplate_blob = merge_mining::serialize_monero_block_to_hex(&monero_block)?;
        debug!(target: LOG_TARGET, "blocktemplate_blob:{}", block_template_blob);
        monerod_resp["result"]["blocktemplate_blob"] = blocktemplate_blob.into();

        let seed = monerod_resp["result"]["seed_hash"].to_string().replace("\"", "");

        let block_data = block_data.monero_seed(seed);

        let monero_difficulty = monerod_resp["result"]["difficulty"].as_u64().unwrap_or_default();

        let mining_difficulty = cmp::min(monero_difficulty, tari_difficulty);

        let block_data = block_data
            .monero_difficulty(monero_difficulty)
            .tari_difficulty(tari_difficulty);

        info!(
            target: LOG_TARGET,
            "Difficulties: Tari ({}), Monero({}), Selected({})", tari_difficulty, monero_difficulty, mining_difficulty
        );
        monerod_resp["result"]["difficulty"] = mining_difficulty.into();
        let monerod_resp = helpers::add_aux_data(monerod_resp, json!({ "base_difficulty": monero_difficulty }));
        let monerod_resp = helpers::append_aux_chain_data(
            monerod_resp,
            json!({
                "id": TARI_CHAIN_ID,
                "difficulty": tari_difficulty,
                "height": tari_height,
                // The merge mining hash, before the final block hash can be calculated
                "mining_hash": mining_hash.to_hex(),
                "miner_reward": block_reward + total_fees,
            }),
        );

        self.block_templates.save(mining_hash, block_data.build()?).await;

        debug!(target: LOG_TARGET, "Returning template result: {}", monerod_resp);
        Ok(proxy::into_response(parts, &monerod_resp))
    }
}
