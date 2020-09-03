// Copyright 2020. The Tari Project
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

use crate::tari_rpc as grpc;
use prost_types::Timestamp;
use std::convert::{TryFrom, TryInto};
use tari_core::{
    blocks::{Block, BlockHeader, NewBlockHeaderTemplate, NewBlockTemplate},
    chain_storage::HistoricalBlock,
    proof_of_work::{Difficulty, PowAlgorithm, ProofOfWork},
    transactions::types::BlindingFactor,
};
use tari_crypto::tari_utilities::{epoch_time::EpochTime, ByteArray, Hashable};

impl TryFrom<grpc::ProofOfWork> for ProofOfWork {
    type Error = String;

    fn try_from(pow: grpc::ProofOfWork) -> Result<Self, Self::Error> {
        Ok(Self {
            pow_algo: PowAlgorithm::try_from(pow.pow_algo)?,
            accumulated_monero_difficulty: Difficulty::from(pow.accumulated_monero_difficulty),
            accumulated_blake_difficulty: Difficulty::from(pow.accumulated_blake_difficulty),
            target_difficulty: Difficulty::from(pow.target_difficulty),
            pow_data: pow.pow_data,
        })
    }
}
