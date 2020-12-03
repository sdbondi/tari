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
    blocks::{Block, BlockValidationError},
    chain_storage::{async_db::AsyncBlockchainDb, BlockchainBackend},
    consensus::{ConsensusConstants, ConsensusManager},
    tari_utilities::{hex::Hex, Hashable},
    transactions::{transaction::TransactionError, types::CryptoFactories},
    validation::{helpers::check_block_weight, ValidationError},
};
use asparit::IntoParallelIterator;

const LOG_TARGET: &str = "c::bn::block_sync::validator";

/// This validator checks whether a block satisfies consensus rules.
/// It implements a validator for `Block`. The `Block` validator ONLY validates the block body (but still requires the
/// header to validate against). It is assumed that the `BlockHeader` has been previously validated (e.g. in header
/// sync).
pub struct BlockValidator<B> {
    db: AsyncBlockchainDb<B>,
    rules: ConsensusManager,
    factories: CryptoFactories,
}

impl<B: BlockchainBackend> BlockValidator<B> {
    pub fn new(db: AsyncBlockchainDb<B>, rules: ConsensusManager, factories: CryptoFactories) -> Self {
        Self { db, rules, factories }
    }

    /// This function checks that all inputs in the blocks are valid UTXO's to be spend
    fn check_inputs(&self, block: &Block) -> Result<(), ValidationError> {
        for input in block.body.inputs() {
            // Check maturity
            if input.features.maturity > block.header.height {
                warn!(
                    target: LOG_TARGET,
                    "Input found that has not yet matured to spending height: {}", input
                );
                return Err(TransactionError::InputMaturity.into());
            }

            // Check that the block body has cut-through applied
            if block.body.outputs().iter().any(|o| o.is_equal_to(input)) {
                warn!(
                    target: LOG_TARGET,
                    "Block #{} failed to validate: block no cut through", block.header.height
                );
                return Err(BlockValidationError::NoCutThrough.into());
            }
        }

        Ok(())
    }

    fn check_outputs(&self, block: &Block, constants: &ConsensusConstants) -> Result<(), ValidationError> {
        let total_coinbase = self.rules.calculate_coinbase_and_fees(block);
        block.check_coinbase_output(total_coinbase, &constants, &self.factories)?;
        trace!(
            target: LOG_TARGET,
            "SV - Coinbase output is ok for #{} ",
            &block.header.height
        );

        Ok(())
    }

    fn check_mmr_roots(&self, db: &B, block: &Block) -> Result<(), ValidationError> {
        let header = &block.header;
        if header.kernel_mr != mmr_roots.kernel_mr {
            warn!(
                target: LOG_TARGET,
                "Block header kernel MMR roots in {} do not match calculated roots",
                block.hash().to_hex()
            );
            return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrRoots));
        }
        if header.output_mr != mmr_roots.output_mr {
            warn!(
                target: LOG_TARGET,
                "Block header output MMR roots in {} do not match calculated roots",
                block.hash().to_hex()
            );
            return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrRoots));
        }
        if header.range_proof_mr != mmr_roots.range_proof_mr {
            warn!(
                target: LOG_TARGET,
                "Block header range_proof MMR roots in {} do not match calculated roots",
                block.hash().to_hex()
            );
            return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrRoots));
        }
        Ok(())
    }

    pub async fn validate(&self, block: &Block) -> Result<(), ValidationError> {
        let height = block.header.height;
        let block_id = format!("block #{}", height);
        trace!(target: LOG_TARGET, "Validating {}", block_id);

        let constants = self.rules.consensus_constants(height);
        check_block_weight(&block, &constants)?;
        trace!(target: LOG_TARGET, "SV - Block weight is ok for {} ", &block_id);

        self.check_inputs(&block)?;
        self.check_outputs(&block, constants)?;

        // self.check_accounting_balace

        let a = block
            .body
            .kernels()
            .into_par_iter()
            .map(|k| k.verify_signature())
            .exec()
            .await;

        // check_accounting_balance(block, &self.rules, &self.factories)?;
        trace!(target: LOG_TARGET, "SV - accounting balance correct for {}", &block_id);
        debug!(
            target: LOG_TARGET,
            "{} has PASSED stateless VALIDATION check.", &block_id
        );

        let mmr_roots = db.calculate_mmr_roots(block).await?;
        self.check_mmr_roots(&*db, &block)?;
        trace!(
            target: LOG_TARGET,
            "Block validation: MMR roots are valid for {}",
            block_id
        );

        debug!(target: LOG_TARGET, "Block validation: Block is VALID for {}", block_id);
        Ok(())
    }
}

impl<B: BlockchainBackend> Validation<Block> for BlockValidator<B> {
    /// The following consensus checks are done
    /// 1. Does the block satisfy the stateless checks?
    /// 1. Are the block header MMR roots valid?
    fn validate(&self, block: &Block) -> Result<(), ValidationError> {
        let block_id = format!("block #{}", block.header.height);
        trace!(target: LOG_TARGET, "Validating {}", block_id);

        let constants = self.rules.consensus_constants(block.header.height);
        check_block_weight(&block, &constants)?;
        trace!(target: LOG_TARGET, "SV - Block weight is ok for {} ", &block_id);

        self.check_inputs(&block)?;
        self.check_outputs(&block, constants)?;

        check_accounting_balance(block, &self.rules, &self.factories)?;
        trace!(target: LOG_TARGET, "SV - accounting balance correct for {}", &block_id);
        debug!(
            target: LOG_TARGET,
            "{} has PASSED stateless VALIDATION check.", &block_id
        );

        let mmr_roots = db.calculate_mmr_roots(block).await?;
        self.check_mmr_roots(&*db, &block)?;
        trace!(
            target: LOG_TARGET,
            "Block validation: MMR roots are valid for {}",
            block_id
        );

        debug!(target: LOG_TARGET, "Block validation: Block is VALID for {}", block_id);
        Ok(())
    }
}
