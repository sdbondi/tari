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

use crate::{
    blocks::{
        block_header::{BlockHeader, BlockHeaderValidationError},
        Block,
        BlockValidationError,
    },
    chain_storage,
    chain_storage::{BlockchainBackend, MmrTree},
    consensus::{ConsensusConstants, ConsensusManager},
    proof_of_work::{
        monero_difficulty,
        monero_rx::MoneroPowData,
        randomx_factory::RandomXFactory,
        sha3_difficulty,
        AchievedTargetDifficulty,
        Difficulty,
        PowAlgorithm,
        PowError,
    },
    transactions::{aggregated_body::AggregateBody, types::CryptoFactories},
    validation::ValidationError,
};
use log::*;
use tari_crypto::tari_utilities::{epoch_time::EpochTime, hash::Hashable, hex::Hex};

pub const LOG_TARGET: &str = "c::val::helpers";

/// This function tests that the block timestamp is less than the FTL
pub fn check_timestamp_ftl(
    block_header: &BlockHeader,
    consensus_manager: &ConsensusManager,
) -> Result<(), ValidationError> {
    if block_header.timestamp > consensus_manager.consensus_constants(block_header.height).ftl() {
        warn!(
            target: LOG_TARGET,
            "Invalid Future Time Limit on block:{}",
            block_header.hash().to_hex()
        );
        return Err(ValidationError::BlockHeaderError(
            BlockHeaderValidationError::InvalidTimestampFutureTimeLimit,
        ));
    }
    Ok(())
}

/// Returns the median timestamp for the provided timestamps.
pub fn calc_median_timestamp(timestamps: &[EpochTime]) -> EpochTime {
    assert!(
        !timestamps.is_empty(),
        "calc_median_timestamp: timestamps cannot be empty"
    );
    trace!(
        target: LOG_TARGET,
        "Calculate the median timestamp from {} timestamps",
        timestamps.len()
    );

    let mid_index = timestamps.len() / 2;
    let median_timestamp = if timestamps.len() % 2 == 0 {
        (timestamps[mid_index - 1] + timestamps[mid_index]) / 2
    } else {
        timestamps[mid_index]
    };
    trace!(target: LOG_TARGET, "Median timestamp:{}", median_timestamp);
    median_timestamp
}

pub fn check_header_timestamp_greater_than_median(
    block_header: &BlockHeader,
    timestamps: &[EpochTime],
) -> Result<(), ValidationError> {
    if timestamps.is_empty() {
        return Err(ValidationError::BlockHeaderError(
            BlockHeaderValidationError::InvalidTimestamp("The timestamp is empty".to_string()),
        ));
    }

    let median_timestamp = calc_median_timestamp(timestamps);
    if block_header.timestamp < median_timestamp {
        warn!(
            target: LOG_TARGET,
            "Block header timestamp {} is less than median timestamp: {} for block:{}",
            block_header.timestamp,
            median_timestamp,
            block_header.hash().to_hex()
        );
        return Err(ValidationError::BlockHeaderError(
            BlockHeaderValidationError::InvalidTimestamp(format!(
                "The timestamp `{}` was less than the median timestamp `{}`",
                block_header.timestamp, median_timestamp
            )),
        ));
    }

    Ok(())
}

/// Check the PoW data in the BlockHeader. This currently only applies to blocks merged mined with Monero.
pub fn check_pow_data<B: BlockchainBackend>(
    block_header: &BlockHeader,
    rules: &ConsensusManager,
    db: &B,
) -> Result<(), ValidationError> {
    use PowAlgorithm::*;
    match block_header.pow.pow_algo {
        Monero => {
            let monero_data =
                MoneroPowData::from_header(block_header).map_err(|e| ValidationError::CustomError(e.to_string()))?;
            let seed_height = db.fetch_monero_seed_first_seen_height(&monero_data.randomx_key)?;
            if (seed_height != 0) &&
                (block_header.height - seed_height >
                    rules.consensus_constants(block_header.height).max_randomx_seed_height())
            {
                return Err(ValidationError::BlockHeaderError(
                    BlockHeaderValidationError::OldSeedHash,
                ));
            }

            Ok(())
        },
        Sha3 => {
            if !block_header.pow.pow_data.is_empty() {
                return Err(ValidationError::CustomError(
                    "Proof of work data must be empty for Sha3 blocks".to_string(),
                ));
            }
            Ok(())
        },
    }
}

pub fn check_target_difficulty(
    block_header: &BlockHeader,
    target: Difficulty,
    randomx_factory: &RandomXFactory,
) -> Result<AchievedTargetDifficulty, ValidationError> {
    let achieved = match block_header.pow_algo() {
        PowAlgorithm::Monero => monero_difficulty(block_header, randomx_factory)?,
        PowAlgorithm::Sha3 => sha3_difficulty(block_header),
    };

    match AchievedTargetDifficulty::try_construct(block_header.pow_algo(), target, achieved) {
        Some(achieved_target) => Ok(achieved_target),
        None => {
            warn!(
                target: LOG_TARGET,
                "Proof of work for {} at height {} was below the target difficulty. Achieved: {}, Target: {}",
                block_header.hash().to_hex(),
                block_header.height,
                achieved,
                target
            );
            Err(ValidationError::BlockHeaderError(
                BlockHeaderValidationError::ProofOfWorkError(PowError::AchievedDifficultyTooLow { achieved, target }),
            ))
        },
    }
}

pub fn check_block_weight(block: &Block, consensus_constants: &ConsensusConstants) -> Result<(), ValidationError> {
    // The genesis block has a larger weight than other blocks may have so we have to exclude it here
    let block_weight = block.body.calculate_weight();
    if block_weight <= consensus_constants.get_max_block_transaction_weight() || block.header.height == 0 {
        trace!(
            target: LOG_TARGET,
            "SV - Block contents for block #{} : {}; weight {}.",
            block.header.height,
            block.body.to_counts_string(),
            block_weight,
        );

        Ok(())
    } else {
        Err(BlockValidationError::BlockTooLarge).map_err(ValidationError::from)
    }
}

pub fn check_accounting_balance(
    block: &Block,
    rules: &ConsensusManager,
    factories: &CryptoFactories,
) -> Result<(), ValidationError> {
    if block.header.height == 0 {
        // Gen block does not need to be checked for this.
        return Ok(());
    }
    let offset = &block.header.total_kernel_offset;
    let script_offset = &block.header.total_script_offset;
    let total_coinbase = rules.calculate_coinbase_and_fees(block);
    block
        .body
        .validate_internal_consistency(&offset, &script_offset, total_coinbase, factories)
        .map_err(|err| {
            warn!(
                target: LOG_TARGET,
                "Validation failed on block:{}:{}",
                block.hash().to_hex(),
                err
            );
            ValidationError::TransactionError(err)
        })
}

pub fn check_coinbase_output(
    block: &Block,
    rules: &ConsensusManager,
    factories: &CryptoFactories,
) -> Result<(), ValidationError> {
    let total_coinbase = rules.calculate_coinbase_and_fees(block);
    block
        .check_coinbase_output(
            total_coinbase,
            rules.consensus_constants(block.header.height),
            factories,
        )
        .map_err(ValidationError::from)
}

fn is_all_unique_and_sorted<I: AsRef<[T]>, T: PartialOrd>(items: I) -> bool {
    let items = items.as_ref();
    if items.is_empty() {
        return true;
    }

    let mut prev_item = &items[0];
    for item in items.iter().skip(1) {
        if item <= prev_item {
            return false;
        }
        prev_item = &item;
    }

    true
}

// This function checks for duplicate inputs and outputs. There should be no duplicate inputs or outputs in a block
pub fn check_sorting_and_duplicates(body: &AggregateBody) -> Result<(), ValidationError> {
    if !is_all_unique_and_sorted(body.inputs()) {
        return Err(ValidationError::UnsortedOrDuplicateInput);
    }
    if !is_all_unique_and_sorted(body.outputs()) {
        return Err(ValidationError::UnsortedOrDuplicateOutput);
    }

    Ok(())
}

/// This function checks that all inputs in the blocks are valid UTXO's to be spent
pub fn check_inputs_are_utxos<B: BlockchainBackend>(body: &AggregateBody, db: &B) -> Result<(), ValidationError> {
    let mut not_found_input = Vec::new();
    for input in body.inputs() {
        let output_hash = input.output_hash();
        if let Some(utxo_hash) = db.fetch_unspent_output_hash_by_commitment(&input.commitment)? {
            // We know that the commitment exists in the UTXO set. Check that the output hash matches (i.e. all fields
            // like output features match)
            if utxo_hash == output_hash {
                continue;
            }

            warn!(
                target: LOG_TARGET,
                "Input spends a UTXO but does not produce the same hash as the output it spends:
            {}",
                input
            );
            return Err(ValidationError::BlockError(BlockValidationError::InvalidInput));
        }

        // Wallet needs to know if a transaction has already been mined and uses this error variant to do so.
        if db.fetch_output(&output_hash)?.is_some() {
            warn!(
                target: LOG_TARGET,
                "Validation failed due to already spent input: {}", input
            );
            // We know that the output here must be spent because `fetch_unspent_output_hash_by_commitment` would have
            // been Some
            return Err(ValidationError::ContainsSTxO);
        }

        if body.outputs().iter().any(|output| output.hash() == output_hash) {
            continue;
        }

        warn!(
            target: LOG_TARGET,
            "Validation failed due to input: {} which does not exist yet", input
        );
        not_found_input.push(output_hash);
    }
    if !not_found_input.is_empty() {
        return Err(ValidationError::UnknownInputs(not_found_input));
    }

    Ok(())
}

/// This function checks that the outputs do not already exist in the UTxO set.
pub fn check_not_duplicate_txos<B: BlockchainBackend>(body: &AggregateBody, db: &B) -> Result<(), ValidationError> {
    for output in body.outputs() {
        if let Some(index) = db.fetch_mmr_leaf_index(MmrTree::Utxo, &output.hash())? {
            warn!(
                target: LOG_TARGET,
                "Validation failed due to previously spent output: {} (MMR index = {})", output, index
            );
            return Err(ValidationError::ContainsTxO);
        }
        if db
            .fetch_unspent_output_hash_by_commitment(&output.commitment)?
            .is_some()
        {
            warn!(
                target: LOG_TARGET,
                "Duplicate UTXO set commitment found for output: {}", output
            );
            return Err(ValidationError::ContainsDuplicateUtxoCommitment);
        }
    }
    Ok(())
}

pub fn check_mmr_roots<B: BlockchainBackend>(block: &Block, db: &B) -> Result<(), ValidationError> {
    let mmr_roots = chain_storage::calculate_mmr_roots(db, &block)?;
    let header = &block.header;
    if header.input_mr != mmr_roots.input_mr {
        warn!(
            target: LOG_TARGET,
            "Block header input merkle root in {} do not match calculated root. Expected: {}, Actual:{}",
            block.hash().to_hex(),
            header.input_mr.to_hex(),
            mmr_roots.input_mr.to_hex()
        );
        return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrRoots));
    }
    if header.kernel_mr != mmr_roots.kernel_mr {
        warn!(
            target: LOG_TARGET,
            "Block header kernel MMR roots in {} do not match calculated roots. Expected: {}, Actual:{}",
            block.hash().to_hex(),
            header.kernel_mr.to_hex(),
            mmr_roots.kernel_mr.to_hex()
        );
        return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrRoots));
    };
    if header.kernel_mmr_size != mmr_roots.kernel_mmr_size {
        warn!(
            target: LOG_TARGET,
            "Block header kernel MMR size in {} does not match. Expected: {}, Actual:{}",
            block.hash().to_hex(),
            header.kernel_mmr_size,
            mmr_roots.kernel_mmr_size
        );
        return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrSize {
            mmr_tree: MmrTree::Kernel,
            expected: mmr_roots.kernel_mmr_size,
            actual: header.kernel_mmr_size,
        }));
    }
    if header.output_mr != mmr_roots.output_mr {
        warn!(
            target: LOG_TARGET,
            "Block header output MMR roots in {} do not match calculated roots. Expected: {}, Actual:{}",
            block.hash().to_hex(),
            header.output_mr.to_hex(),
            mmr_roots.output_mr.to_hex()
        );
        return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrRoots));
    };
    if header.witness_mr != mmr_roots.witness_mr {
        warn!(
            target: LOG_TARGET,
            "Block header witness MMR roots in {} do not match calculated roots",
            block.hash().to_hex()
        );
        return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrRoots));
    };
    if header.output_mmr_size != mmr_roots.output_mmr_size {
        warn!(
            target: LOG_TARGET,
            "Block header output MMR size in {} does not match. Expected: {}, Actual:{}",
            block.hash().to_hex(),
            header.output_mmr_size,
            mmr_roots.output_mmr_size
        );
        return Err(ValidationError::BlockError(BlockValidationError::MismatchedMmrSize {
            mmr_tree: MmrTree::Utxo,
            expected: mmr_roots.output_mmr_size,
            actual: header.output_mmr_size,
        }));
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn it_returns_true_when_nothing_to_compare() {
        assert!(is_all_unique_and_sorted::<_, usize>(&[]));
        assert!(is_all_unique_and_sorted(&[1]));
    }
    #[test]
    fn it_returns_true_when_unique_and_sorted() {
        let v = [1, 2, 3, 4, 5];
        assert!(is_all_unique_and_sorted(&v));
    }

    #[test]
    fn it_returns_false_when_unsorted() {
        let v = [2, 1, 3, 4, 5];
        assert!(!is_all_unique_and_sorted(&v));
    }
    #[test]
    fn it_returns_false_when_duplicate() {
        let v = [1, 2, 3, 4, 4];
        assert!(!is_all_unique_and_sorted(&v));
    }
    #[test]
    fn it_returns_false_when_duplicate_and_unsorted() {
        let v = [4, 2, 3, 0, 4];
        assert!(!is_all_unique_and_sorted(&v));
    }

    // #[allow(clippy::module_inception)]
    #[test]
    #[should_panic]
    fn it_panics_if_empty() {
        calc_median_timestamp(&[]);
    }

    #[test]
    fn it_calculates_the_correct_median_timestamp() {
        let median_timestamp = calc_median_timestamp(&[0.into()]);
        assert_eq!(median_timestamp, 0.into());

        let median_timestamp = calc_median_timestamp(&[123.into()]);
        assert_eq!(median_timestamp, 123.into());

        let median_timestamp = calc_median_timestamp(&[2.into(), 4.into()]);
        assert_eq!(median_timestamp, 3.into());

        let median_timestamp = calc_median_timestamp(&[0.into(), 100.into(), 0.into()]);
        assert_eq!(median_timestamp, 100.into());

        let median_timestamp = calc_median_timestamp(&[1.into(), 2.into(), 3.into(), 4.into()]);
        assert_eq!(median_timestamp, 2.into());

        let median_timestamp = calc_median_timestamp(&[1.into(), 2.into(), 3.into(), 4.into(), 5.into()]);
        assert_eq!(median_timestamp, 3.into());
    }
}
