// Copyright 2019, The Tari Project
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
    proto,
    tari_utilities::convert::try_convert_all,
    transactions::{
        aggregated_body::AggregateBody,
        bullet_rangeproofs::BulletRangeProof,
        tari_amount::MicroTari,
        transaction::{
            KernelFeatures,
            OutputFeatures,
            OutputFlags,
            Transaction,
            TransactionInput,
            TransactionKernel,
            TransactionOutput,
        },
        types::{BlindingFactor, Commitment, HashOutput, PrivateKey, PublicKey, Signature},
    },
};
use std::convert::{TryFrom, TryInto};
use tari_crypto::tari_utilities::{ByteArray, ByteArrayError};

//---------------------------------- Commitment --------------------------------------------//

impl TryFrom<proto::types::Commitment> for Commitment {
    type Error = ByteArrayError;

    fn try_from(commitment: proto::types::Commitment) -> Result<Self, Self::Error> {
        Commitment::from_bytes(&commitment.data)
    }
}

impl From<Commitment> for proto::types::Commitment {
    fn from(commitment: Commitment) -> Self {
        Self {
            data: commitment.to_vec(),
        }
    }
}

//---------------------------------- Signature --------------------------------------------//

impl TryFrom<proto::types::Signature> for Signature {
    type Error = ByteArrayError;

    fn try_from(sig: proto::types::Signature) -> Result<Self, Self::Error> {
        let public_nonce = PublicKey::from_bytes(&sig.public_nonce)?;
        let signature = PrivateKey::from_bytes(&sig.signature)?;

        Ok(Self::new(public_nonce, signature))
    }
}

impl From<Signature> for proto::types::Signature {
    fn from(sig: Signature) -> Self {
        Self {
            public_nonce: sig.get_public_nonce().to_vec(),
            signature: sig.get_signature().to_vec(),
        }
    }
}

//---------------------------------- HashOutput --------------------------------------------//

impl From<proto::types::HashOutput> for HashOutput {
    fn from(output: proto::types::HashOutput) -> Self {
        output.data
    }
}

impl From<HashOutput> for proto::types::HashOutput {
    fn from(output: HashOutput) -> Self {
        Self { data: output }
    }
}

//--------------------------------- BlindingFactor -----------------------------------------//

impl TryFrom<proto::types::BlindingFactor> for BlindingFactor {
    type Error = ByteArrayError;

    fn try_from(offset: proto::types::BlindingFactor) -> Result<Self, Self::Error> {
        Ok(BlindingFactor::from_bytes(&offset.data)?)
    }
}

impl From<BlindingFactor> for proto::types::BlindingFactor {
    fn from(offset: BlindingFactor) -> Self {
        Self { data: offset.to_vec() }
    }
}

//---------------------------------- TransactionKernel --------------------------------------------//

impl TryFrom<proto::types::TransactionKernel> for TransactionKernel {
    type Error = String;

    fn try_from(kernel: proto::types::TransactionKernel) -> Result<Self, Self::Error> {
        let excess = Commitment::from_bytes(
            &kernel
                .excess
                .ok_or_else(|| "Excess not provided in kernel".to_string())?
                .data,
        )
        .map_err(|err| err.to_string())?;

        let excess_sig = kernel
            .excess_sig
            .ok_or_else(|| "excess_sig not provided".to_string())?
            .try_into()
            .map_err(|err: ByteArrayError| err.to_string())?;

        Ok(Self {
            features: KernelFeatures::from_bits(kernel.features as u8)
                .ok_or_else(|| "Invalid or unrecognised kernel feature flag".to_string())?,
            excess,
            excess_sig,
            fee: MicroTari::from(kernel.fee),
            lock_height: kernel.lock_height,
        })
    }
}

impl From<TransactionKernel> for proto::types::TransactionKernel {
    fn from(kernel: TransactionKernel) -> Self {
        Self {
            features: kernel.features.bits() as u32,
            excess: Some(kernel.excess.into()),
            excess_sig: Some(kernel.excess_sig.into()),
            fee: kernel.fee.into(),
            lock_height: kernel.lock_height,
        }
    }
}

//---------------------------------- TransactionInput --------------------------------------------//

impl TryFrom<proto::types::TransactionInput> for TransactionInput {
    type Error = String;

    fn try_from(input: proto::types::TransactionInput) -> Result<Self, Self::Error> {
        let features = input
            .features
            .map(TryInto::try_into)
            .ok_or_else(|| "transaction output features not provided".to_string())??;

        let commitment = input
            .commitment
            .map(|commit| Commitment::from_bytes(&commit.data))
            .ok_or_else(|| "Transaction output commitment not provided".to_string())?
            .map_err(|err| err.to_string())?;

        Ok(Self { features, commitment })
    }
}

impl From<TransactionInput> for proto::types::TransactionInput {
    fn from(output: TransactionInput) -> Self {
        Self {
            features: Some(output.features.into()),
            commitment: Some(output.commitment.into()),
        }
    }
}

//---------------------------------- TransactionOutput --------------------------------------------//

impl TryFrom<proto::types::TransactionOutput> for TransactionOutput {
    type Error = String;

    fn try_from(output: proto::types::TransactionOutput) -> Result<Self, Self::Error> {
        let features = output
            .features
            .map(TryInto::try_into)
            .ok_or_else(|| "transaction output features not provided".to_string())??;

        let commitment = output
            .commitment
            .map(|commit| Commitment::from_bytes(&commit.data))
            .ok_or_else(|| "Transaction output commitment not provided".to_string())?
            .map_err(|err| err.to_string())?;

        Ok(Self {
            features,
            commitment,
            proof: BulletRangeProof(output.range_proof),
        })
    }
}

impl From<TransactionOutput> for proto::types::TransactionOutput {
    fn from(output: TransactionOutput) -> Self {
        Self {
            features: Some(output.features.into()),
            commitment: Some(output.commitment.into()),
            range_proof: output.proof.to_vec(),
        }
    }
}

//---------------------------------- OutputFeatures --------------------------------------------//

impl TryFrom<proto::types::OutputFeatures> for OutputFeatures {
    type Error = String;

    fn try_from(features: proto::types::OutputFeatures) -> Result<Self, Self::Error> {
        Ok(Self {
            flags: OutputFlags::from_bits(features.flags as u8)
                .ok_or_else(|| "Invalid or unrecognised output flags".to_string())?,
            maturity: features.maturity,
        })
    }
}

impl From<OutputFeatures> for proto::types::OutputFeatures {
    fn from(features: OutputFeatures) -> Self {
        Self {
            flags: features.flags.bits() as u32,
            maturity: features.maturity,
        }
    }
}

//---------------------------------- AggregateBody --------------------------------------------//

impl TryFrom<proto::types::AggregateBody> for AggregateBody {
    type Error = String;

    fn try_from(body: proto::types::AggregateBody) -> Result<Self, Self::Error> {
        let inputs = try_convert_all(body.inputs)?;
        let outputs = try_convert_all(body.outputs)?;
        let kernels = try_convert_all(body.kernels)?;
        let mut body = AggregateBody::new(inputs, outputs, kernels);
        body.sort();
        Ok(body)
    }
}

impl From<AggregateBody> for proto::types::AggregateBody {
    fn from(body: AggregateBody) -> Self {
        let (i, o, k) = body.dissolve();
        Self {
            inputs: i.into_iter().map(Into::into).collect(),
            outputs: o.into_iter().map(Into::into).collect(),
            kernels: k.into_iter().map(Into::into).collect(),
        }
    }
}

//----------------------------------- Transaction ---------------------------------------------//

impl TryFrom<proto::types::Transaction> for Transaction {
    type Error = String;

    fn try_from(tx: proto::types::Transaction) -> Result<Self, Self::Error> {
        let offset = tx
            .offset
            .map(|offset| BlindingFactor::from_bytes(&offset.data))
            .ok_or_else(|| "Blinding factor offset not provided".to_string())?
            .map_err(|err| err.to_string())?;
        let body = tx
            .body
            .map(TryInto::try_into)
            .ok_or_else(|| "Body not provided".to_string())??;

        Ok(Self { offset, body })
    }
}

impl From<Transaction> for proto::types::Transaction {
    fn from(tx: Transaction) -> Self {
        Self {
            offset: Some(tx.offset.into()),
            body: Some(tx.body.into()),
        }
    }
}
