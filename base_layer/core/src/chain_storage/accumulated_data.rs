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
    blocks::{Block, BlockHeader},
    chain_storage::ChainStorageError,
    proof_of_work::{Difficulty, PowAlgorithm},
    transactions::types::{BlindingFactor, Commitment, HashOutput},
};
use croaring::Bitmap;
use serde::{
    de,
    de::{MapAccess, SeqAccess, Visitor},
    ser::SerializeStruct,
    Deserialize,
    Deserializer,
    Serialize,
    Serializer,
};
use std::fmt;
use tari_mmr::pruned_hashset::PrunedHashSet;

#[derive(Debug)]
pub struct BlockAccumulatedData {
    pub(super) kernels: PrunedHashSet,
    pub(super) outputs: PrunedHashSet,
    pub(super) deleted: Bitmap,
    pub(super) range_proofs: PrunedHashSet,
    pub(super) total_kernel_sum: Commitment,
    pub(super) total_utxo_sum: Commitment,
}

impl BlockAccumulatedData {
    pub fn new(
        kernels: PrunedHashSet,
        outputs: PrunedHashSet,
        range_proofs: PrunedHashSet,
        deleted: Bitmap,
        total_kernel_sum: Commitment,
        total_utxo_sum: Commitment,
    ) -> Self
    {
        Self {
            kernels,
            outputs,
            range_proofs,
            deleted,
            total_kernel_sum,
            total_utxo_sum,
        }
    }

    #[inline(always)]
    pub fn deleted(&self) -> &Bitmap {
        &self.deleted
    }

    pub fn dissolve(self) -> (PrunedHashSet, PrunedHashSet, PrunedHashSet, Bitmap) {
        (self.kernels, self.outputs, self.range_proofs, self.deleted)
    }
}

impl Default for BlockAccumulatedData {
    fn default() -> Self {
        Self {
            kernels: Default::default(),
            outputs: Default::default(),
            deleted: Bitmap::create(),
            range_proofs: Default::default(),
            total_kernel_sum: Default::default(),
            total_utxo_sum: Default::default(),
        }
    }
}

impl Serialize for BlockAccumulatedData {
    fn serialize<S>(&self, serializer: S) -> Result<<S as Serializer>::Ok, <S as Serializer>::Error>
    where S: Serializer {
        let mut s = serializer.serialize_struct("MmrPeakData", 6)?;
        s.serialize_field("kernels", &self.kernels)?;
        s.serialize_field("outputs", &self.outputs)?;
        s.serialize_field("deleted", &self.deleted.serialize())?;
        s.serialize_field("range_proofs", &self.range_proofs)?;
        s.serialize_field("total_kernel_sum", &self.total_kernel_sum)?;
        s.serialize_field("total_utxo_sum", &self.total_utxo_sum)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for BlockAccumulatedData {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
    where D: Deserializer<'de> {
        const FIELDS: &[&str] = &[
            "kernels",
            "outputs",
            "deleted",
            "range_proofs",
            "total_kernel_sum",
            "total_utxo_sum",
        ];

        deserializer.deserialize_struct("MmrPeakData", FIELDS, BlockAccumulatedDataVisitor)
    }
}

struct BlockAccumulatedDataVisitor;

impl<'de> Visitor<'de> for BlockAccumulatedDataVisitor {
    type Value = BlockAccumulatedData;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("`kernels`, `outputs`, `deleted`,`range_proofs`,`total_kernel_sum` or `total_utxo_sum`")
    }

    fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
    where V: SeqAccess<'de> {
        let kernels = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let outputs = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
        let deleted: Vec<u8> = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
        let range_proofs = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
        let total_kernel_sum = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(4, &self))?;
        let total_utxo_sum = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(5, &self))?;
        Ok(BlockAccumulatedData {
            kernels,
            outputs,
            deleted: Bitmap::deserialize(&deleted),
            range_proofs,
            total_kernel_sum,
            total_utxo_sum,
        })
    }

    fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
    where V: MapAccess<'de> {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Kernels,
            Outputs,
            Deleted,
            RangeProofs,
            TotalKernelSum,
            TotalUtxoSum,
        };
        let mut kernels = None;
        let mut outputs = None;
        let mut deleted = None;
        let mut range_proofs = None;
        let mut total_kernel_sum = None;
        let mut total_utxo_sum = None;
        while let Some(key) = map.next_key()? {
            match key {
                Field::Kernels => {
                    if kernels.is_some() {
                        return Err(de::Error::duplicate_field("kernels"));
                    }
                    kernels = Some(map.next_value()?);
                },
                Field::Outputs => {
                    if outputs.is_some() {
                        return Err(de::Error::duplicate_field("outputs"));
                    }
                    outputs = Some(map.next_value()?);
                },
                Field::Deleted => {
                    if deleted.is_some() {
                        return Err(de::Error::duplicate_field("deleted"));
                    }
                    deleted = Some(map.next_value()?);
                },
                Field::RangeProofs => {
                    if range_proofs.is_some() {
                        return Err(de::Error::duplicate_field("range_proofs"));
                    }
                    range_proofs = Some(map.next_value()?);
                },
                Field::TotalKernelSum => {
                    if total_kernel_sum.is_some() {
                        return Err(de::Error::duplicate_field("total_kernel_sum"));
                    }
                    total_kernel_sum = Some(map.next_value()?);
                },
                Field::TotalUtxoSum => {
                    if total_utxo_sum.is_some() {
                        return Err(de::Error::duplicate_field("total_utxo_sum"));
                    }
                    total_utxo_sum = Some(map.next_value()?);
                },
            }
        }
        let kernels = kernels.ok_or_else(|| de::Error::missing_field("kernels"))?;
        let outputs = outputs.ok_or_else(|| de::Error::missing_field("outputs"))?;
        let deleted: Vec<u8> = deleted.ok_or_else(|| de::Error::missing_field("deleted"))?;
        let range_proofs = range_proofs.ok_or_else(|| de::Error::missing_field("range_proofs"))?;
        let total_kernel_sum = total_kernel_sum.ok_or_else(|| de::Error::missing_field("total_kernel_sum"))?;
        let total_utxo_sum = total_utxo_sum.ok_or_else(|| de::Error::missing_field("total_utxo_sum"))?;

        Ok(BlockAccumulatedData {
            kernels,
            outputs,
            deleted: Bitmap::deserialize(&deleted),
            range_proofs,
            total_kernel_sum,
            total_utxo_sum,
        })
    }
}

#[derive(Default)]
pub struct BlockHeaderAccumulatedDataBuilder {
    hash: Option<HashOutput>,
    total_kernel_offset: Option<BlindingFactor>,
    achieved_difficulty: Option<Difficulty>,
    pub accumulated_monero_difficulty: Option<Difficulty>,
    pub accumulated_blake_difficulty: Option<Difficulty>,
    pub target_difficulty: Option<Difficulty>,
}

impl BlockHeaderAccumulatedDataBuilder {
    pub fn hash(mut self, hash: HashOutput) -> Self {
        self.hash = Some(hash);
        self
    }

    pub fn total_kernel_offset(
        mut self,
        previous_kernel_offset: &BlindingFactor,
        current_offset: &BlindingFactor,
    ) -> Self
    {
        self.total_kernel_offset = Some(previous_kernel_offset + current_offset);
        self
    }

    pub fn target_difficulty(mut self, target: Difficulty) -> Self {
        self.target_difficulty = Some(target);
        self
    }

    pub fn achieved_difficulty(
        mut self,
        previous: &BlockHeaderAccumulatedData,
        algo: PowAlgorithm,
        achieved: Difficulty,
    ) -> Self
    {
        match algo {
            PowAlgorithm::Monero => {
                self.accumulated_monero_difficulty = Some(previous.accumulated_monero_difficulty + achieved);
                self.accumulated_blake_difficulty = Some(previous.accumulated_blake_difficulty);
            },
            PowAlgorithm::Blake => unimplemented!(),
            PowAlgorithm::Sha3 => {
                self.accumulated_monero_difficulty = Some(previous.accumulated_monero_difficulty);
                self.accumulated_blake_difficulty = Some(previous.accumulated_blake_difficulty + achieved);
            },
        }
        self.achieved_difficulty = Some(achieved);
        self
    }

    pub fn build(self) -> Result<BlockHeaderAccumulatedData, ChainStorageError> {
        let monero_diff = self
            .accumulated_monero_difficulty
            .ok_or_else(|| ChainStorageError::InvalidOperation("difficulty not provided".to_string()))?;

        let blake_diff = self
            .accumulated_blake_difficulty
            .ok_or_else(|| ChainStorageError::InvalidOperation("difficulty not provided".to_string()))?;

        Ok(BlockHeaderAccumulatedData {
            hash: self
                .hash
                .ok_or_else(|| ChainStorageError::InvalidOperation("hash not provided".to_string()))?,
            total_kernel_offset: self
                .total_kernel_offset
                .ok_or_else(|| ChainStorageError::InvalidOperation("total_kernel_offset not provided".to_string()))?,
            achieved_difficulty: self
                .achieved_difficulty
                .ok_or_else(|| ChainStorageError::InvalidOperation("achieved_difficulty not provided".to_string()))?,
            total_accumulated_difficulty: monero_diff.as_u64() as u128 * blake_diff.as_u64() as u128,
            accumulated_monero_difficulty: monero_diff,
            accumulated_blake_difficulty: blake_diff,
            target_difficulty: self
                .target_difficulty
                .ok_or_else(|| ChainStorageError::InvalidOperation("target difficulty not provided".to_string()))?,
        })
    }
}

// TODO: Find a better name and move into `core::blocks` mod
#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
pub struct BlockHeaderAccumulatedData {
    pub hash: HashOutput,
    pub total_kernel_offset: BlindingFactor,
    pub achieved_difficulty: Difficulty,
    pub total_accumulated_difficulty: u128,
    /// The total accumulated difficulty for each proof of work algorithms for all blocks since Genesis,
    /// but not including this block, tracked separately.
    pub accumulated_monero_difficulty: Difficulty,
    pub accumulated_blake_difficulty: Difficulty,
    /// The target difficulty for solving the current block using the specified proof of work algorithm.
    pub target_difficulty: Difficulty,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChainHeader {
    pub header: BlockHeader,
    pub accumulated_data: BlockHeaderAccumulatedData,
}

impl ChainHeader {
    pub fn height(&self) -> u64 {
        self.header.height
    }

    pub fn hash(&self) -> &HashOutput {
        &self.accumulated_data.hash
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ChainBlock {
    pub accumulated_data: BlockHeaderAccumulatedData,
    pub block: Block,
}

impl ChainBlock {
    pub fn height(&self) -> u64 {
        self.block.header.height
    }

    pub fn hash(&self) -> &HashOutput {
        &self.accumulated_data.hash
    }
}
