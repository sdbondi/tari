// Copyright 2022. The Tari Project
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

use serde::{Deserialize, Serialize};
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_core::transactions::transaction_components::ContractUpdateProposal;
use tari_utilities::hex::Hex;

use super::ConstitutionDefinitionFileFormat;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractUpdateProposalFileFormat {
    pub proposal_id: u64,
    pub signature: SignatureFileFormat,
    pub updated_constitution: ConstitutionDefinitionFileFormat,
}

impl TryFrom<ContractUpdateProposalFileFormat> for ContractUpdateProposal {
    type Error = String;

    fn try_from(value: ContractUpdateProposalFileFormat) -> Result<Self, Self::Error> {
        Ok(Self {
            proposal_id: value.proposal_id,
            signature: value.signature.try_into()?,
            updated_constitution: value.updated_constitution.try_into()?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignatureFileFormat {
    pub public_nonce: String,
    pub signature: String,
}

impl TryFrom<SignatureFileFormat> for Signature {
    type Error = String;

    fn try_from(value: SignatureFileFormat) -> Result<Self, Self::Error> {
        let public_key = PublicKey::from_hex(&value.public_nonce).map_err(|e| format!("{}", e))?;
        let signature = PrivateKey::from_hex(&value.signature).map_err(|e| format!("{}", e))?;

        Ok(Signature::new(public_key, signature))
    }
}

impl Default for SignatureFileFormat {
    fn default() -> Self {
        let default_sig = Signature::default();
        let public_nonce = default_sig.get_public_nonce().to_hex();
        let signature = default_sig.get_signature().to_hex();

        Self {
            public_nonce,
            signature,
        }
    }
}
