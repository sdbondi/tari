//  Copyright 2022. The Tari Project
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

use std::io::{Error, Read, Write};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;

use super::{
    ContractAcceptance,
    ContractAmendment,
    ContractDefinition,
    ContractUpdateProposal,
    ContractUpdateProposalAcceptance,
};
use crate::{
    consensus::{ConsensusDecoding, ConsensusEncoding, ConsensusEncodingSized},
    transactions::transaction_components::{
        side_chain::contract_checkpoint::ContractCheckpoint,
        ContractConstitution,
        TemplateRegistration,
    },
};

#[derive(Debug, Clone, Hash, PartialEq, Deserialize, Serialize, Eq)]
pub struct SideChainFeatures {
    pub contract_id: FixedHash,
    pub definition: Option<ContractDefinition>,
    pub template_registration: Option<TemplateRegistration>,

    pub constitution: Option<ContractConstitution>,
    pub acceptance: Option<ContractAcceptance>,
    pub update_proposal: Option<ContractUpdateProposal>,
    pub update_proposal_acceptance: Option<ContractUpdateProposalAcceptance>,
    pub amendment: Option<ContractAmendment>,
    pub checkpoint: Option<ContractCheckpoint>,
}

impl SideChainFeatures {
    pub fn new(contract_id: FixedHash) -> Self {
        Self::builder(contract_id).finish()
    }

    pub fn builder(contract_id: FixedHash) -> SideChainFeaturesBuilder {
        SideChainFeaturesBuilder::new(contract_id)
    }
}

impl ConsensusEncoding for SideChainFeatures {
    fn consensus_encode<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.contract_id.consensus_encode(writer)?;
        self.definition.consensus_encode(writer)?;
        self.template_registration.consensus_encode(writer)?;
        self.constitution.consensus_encode(writer)?;
        self.acceptance.consensus_encode(writer)?;
        self.update_proposal.consensus_encode(writer)?;
        self.update_proposal_acceptance.consensus_encode(writer)?;
        self.amendment.consensus_encode(writer)?;
        self.checkpoint.consensus_encode(writer)?;

        Ok(())
    }
}

impl ConsensusEncodingSized for SideChainFeatures {}

impl ConsensusDecoding for SideChainFeatures {
    fn consensus_decode<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(Self {
            contract_id: FixedHash::consensus_decode(reader)?,
            definition: ConsensusDecoding::consensus_decode(reader)?,
            template_registration: ConsensusDecoding::consensus_decode(reader)?,
            constitution: ConsensusDecoding::consensus_decode(reader)?,
            acceptance: ConsensusDecoding::consensus_decode(reader)?,
            update_proposal: ConsensusDecoding::consensus_decode(reader)?,
            update_proposal_acceptance: ConsensusDecoding::consensus_decode(reader)?,
            amendment: ConsensusDecoding::consensus_decode(reader)?,
            checkpoint: ConsensusDecoding::consensus_decode(reader)?,
        })
    }
}

pub struct SideChainFeaturesBuilder {
    features: SideChainFeatures,
}

impl SideChainFeaturesBuilder {
    pub fn new(contract_id: FixedHash) -> Self {
        Self {
            features: SideChainFeatures {
                contract_id,
                definition: None,
                template_registration: None,
                constitution: None,
                acceptance: None,
                update_proposal: None,
                update_proposal_acceptance: None,
                amendment: None,
                checkpoint: None,
            },
        }
    }

    pub fn with_template_registration(mut self, template_registration: TemplateRegistration) -> Self {
        self.features.template_registration = Some(template_registration);
        self
    }

    pub fn with_contract_definition(mut self, contract_definition: ContractDefinition) -> Self {
        self.features.definition = Some(contract_definition);
        self
    }

    pub fn with_contract_constitution(mut self, contract_constitution: ContractConstitution) -> Self {
        self.features.constitution = Some(contract_constitution);
        self
    }

    pub fn with_contract_acceptance(mut self, contract_acceptance: ContractAcceptance) -> Self {
        self.features.acceptance = Some(contract_acceptance);
        self
    }

    pub fn with_contract_update_proposal_acceptance(
        mut self,
        contract_update_proposal_acceptance: ContractUpdateProposalAcceptance,
    ) -> Self {
        self.features.update_proposal_acceptance = Some(contract_update_proposal_acceptance);
        self
    }

    pub fn with_update_proposal(mut self, update_proposal: ContractUpdateProposal) -> Self {
        self.features.update_proposal = Some(update_proposal);
        self
    }

    pub fn with_contract_amendment(mut self, contract_amendment: ContractAmendment) -> Self {
        self.features.amendment = Some(contract_amendment);
        self
    }

    pub fn with_contract_checkpoint(mut self, checkpoint: ContractCheckpoint) -> Self {
        self.features.checkpoint = Some(checkpoint);
        self
    }

    pub fn finish(self) -> SideChainFeatures {
        self.features
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryInto;

    use tari_common_types::types::{PublicKey, Signature};
    use tari_utilities::hex::from_hex;

    use super::*;
    use crate::{
        consensus::{check_consensus_encoding_correctness, MaxSizeString},
        transactions::transaction_components::{
            bytes_into_fixed_string,
            BuildInfo,
            CheckpointParameters,
            CommitteeMembers,
            CommitteeSignatures,
            ConstitutionChangeFlags,
            ConstitutionChangeRules,
            ContractAcceptanceRequirements,
            ContractSpecification,
            FunctionRef,
            PublicFunction,
            RequirementsForConstitutionChange,
            SideChainConsensus,
            SignerSignature,
            TemplateType,
        },
    };

    #[allow(clippy::too_many_lines)]
    #[test]
    fn it_encodes_and_decodes_correctly() {
        let constitution = ContractConstitution {
            validator_committee: vec![PublicKey::default(); CommitteeMembers::MAX_MEMBERS]
                .try_into()
                .unwrap(),
            acceptance_requirements: ContractAcceptanceRequirements {
                acceptance_period_expiry: 100,
                minimum_quorum_required: 5,
            },
            consensus: SideChainConsensus::MerkleRoot,
            checkpoint_params: CheckpointParameters {
                minimum_quorum_required: 5,
                abandoned_interval: 100,
                quarantine_interval: 100,
            },
            constitution_change_rules: ConstitutionChangeRules {
                change_flags: ConstitutionChangeFlags::all(),
                requirements_for_constitution_change: Some(RequirementsForConstitutionChange {
                    minimum_constitution_committee_signatures: 5,
                    constitution_committee: Some(
                        vec![PublicKey::default(); CommitteeMembers::MAX_MEMBERS]
                            .try_into()
                            .unwrap(),
                    ),
                    backup_keys: Some(
                        vec![PublicKey::default(); CommitteeMembers::MAX_MEMBERS]
                            .try_into()
                            .unwrap(),
                    ),
                }),
            },
        };

        let subject = SideChainFeatures {
            contract_id: FixedHash::zero(),
            constitution: Some(constitution.clone()),
            template_registration: Some(TemplateRegistration {
                author_public_key: Default::default(),
                author_signature: Default::default(),
                template_name: MaxSizeString::from_str_checked("🚀🚀🚀🚀🚀🚀🚀🚀").unwrap(),
                template_version: 1,
                template_type: TemplateType::Wasm { abi_version: 123 },
                build_info: BuildInfo {
                    repo_url: "/dns/github.com/https/tari_project/wasm_examples".try_into().unwrap(),
                    commit_hash: from_hex("ea29c9f92973fb7eda913902ff6173c62cb1e5df")
                        .unwrap()
                        .try_into()
                        .unwrap(),
                },
                binary_sha: from_hex("c93747637517e3de90839637f0ce1ab7c8a3800b")
                    .unwrap()
                    .try_into()
                    .unwrap(),
                binary_url: "/dns4/github.com/https/tari_project/wasm_examples/releases/download/v0.0.6/coin.zip"
                    .try_into()
                    .unwrap(),
            }),
            definition: Some(ContractDefinition {
                contract_name: bytes_into_fixed_string("name"),
                contract_issuer: PublicKey::default(),
                contract_spec: ContractSpecification {
                    runtime: bytes_into_fixed_string("runtime"),
                    public_functions: vec![
                        PublicFunction {
                            name: bytes_into_fixed_string("foo"),
                            function: FunctionRef {
                                template_id: FixedHash::zero(),
                                function_id: 0_u16,
                            },
                        },
                        PublicFunction {
                            name: bytes_into_fixed_string("bar"),
                            function: FunctionRef {
                                template_id: FixedHash::zero(),
                                function_id: 1_u16,
                            },
                        },
                    ],
                },
            }),
            acceptance: Some(ContractAcceptance {
                validator_node_public_key: PublicKey::default(),
                signature: Signature::default(),
            }),
            update_proposal: Some(ContractUpdateProposal {
                proposal_id: 0_u64,
                signature: Signature::default(),
                updated_constitution: constitution.clone(),
            }),
            update_proposal_acceptance: Some(ContractUpdateProposalAcceptance {
                proposal_id: 0_u64,
                validator_node_public_key: PublicKey::default(),
                signature: Signature::default(),
            }),
            amendment: Some(ContractAmendment {
                proposal_id: 0_u64,
                validator_committee: vec![PublicKey::default(); CommitteeMembers::MAX_MEMBERS]
                    .try_into()
                    .unwrap(),
                validator_signatures: vec![SignerSignature::default(); CommitteeSignatures::MAX_SIGNATURES]
                    .try_into()
                    .unwrap(),
                updated_constitution: constitution,
                activation_window: 0_u64,
            }),
            checkpoint: Some(ContractCheckpoint {
                checkpoint_number: u64::MAX,
                merkle_root: FixedHash::zero(),
                signatures: vec![SignerSignature::default(); 512].try_into().unwrap(),
            }),
        };

        check_consensus_encoding_correctness(subject).unwrap();
    }
}
