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

use std::convert::{TryFrom, TryInto};
use tari_core::{
    proto::utils::try_convert_all,
    transactions::{
        aggregated_body::AggregateBody,
        bullet_rangeproofs::BulletRangeProof,
        tari_amount::MicroTari,
        transaction::{
            KernelFeatures,
            OutputFeatures,
            OutputFlags,
            TransactionInput,
            TransactionKernel,
            TransactionOutput,
        },
        types::{Commitment, PrivateKey, PublicKey, Signature},
    },
};
use tari_crypto::tari_utilities::ByteArray;

use crate::generated::tari_rpc as grpc;
use tari_core::transactions::transaction::Transaction;

impl From<AggregateBody> for grpc::AggregateBody {
    fn from(source: AggregateBody) -> Self {
        Self {
            inputs: source
                .inputs()
                .iter()
                .map(|input| grpc::TransactionInput {
                    features: Some(grpc::OutputFeatures {
                        flags: input.features.flags.bits() as u32,
                        maturity: input.features.maturity,
                    }),
                    commitment: Vec::from(input.commitment.as_bytes()),
                })
                .collect(),
            outputs: source
                .outputs()
                .iter()
                .map(|output| grpc::TransactionOutput {
                    features: Some(grpc::OutputFeatures {
                        flags: output.features.flags.bits() as u32,
                        maturity: output.features.maturity,
                    }),
                    commitment: Vec::from(output.commitment.as_bytes()),
                    range_proof: Vec::from(output.proof.as_bytes()),
                })
                .collect(),
            kernels: source
                .kernels()
                .iter()
                .map(|kernel| grpc::TransactionKernel {
                    features: kernel.features.bits() as u32,
                    fee: kernel.fee.0,
                    lock_height: kernel.lock_height,
                    meta_info: kernel.meta_info.as_ref().cloned().unwrap_or_default(),
                    linked_kernel: kernel.linked_kernel.as_ref().cloned().unwrap_or_default(),
                    excess: Vec::from(kernel.excess.as_bytes()),
                    excess_sig: Some(grpc::Signature {
                        public_nonce: Vec::from(kernel.excess_sig.get_public_nonce().as_bytes()),
                        signature: Vec::from(kernel.excess_sig.get_signature().as_bytes()),
                    }),
                })
                .collect(),
        }
    }
}

impl TryFrom<grpc::AggregateBody> for AggregateBody {
    type Error = String;

    fn try_from(body: grpc::AggregateBody) -> Result<Self, Self::Error> {
        let inputs = try_convert_all(body.inputs)?;
        let outputs = try_convert_all(body.outputs)?;
        let kernels = try_convert_all(body.kernels)?;
        let mut body = AggregateBody::new(inputs, outputs, kernels);
        Ok(body)
    }
}
