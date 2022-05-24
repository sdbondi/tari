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

use std::mem;

use blake2::VarBlake2b;
use digest::{Update, VariableOutput};
use tari_common_types::types::PublicKey;
use tari_crypto::ristretto::pedersen::PedersenCommitment;
use tari_utilities::ByteArray;

use crate::transactions::{
    transaction_components::{OutputFeatures, OutputFeaturesVersion, OutputFlags, TemplateParameter},
    transaction_protocol::RewindData,
};

pub struct OutputFeaturesBuilder {
    features: OutputFeatures,
}

impl OutputFeaturesBuilder {
    pub fn new() -> Self {
        Self {
            features: OutputFeatures::default(),
        }
    }

    pub fn with_version(mut self, version: OutputFeaturesVersion) -> Self {
        self.features.version = version;
        self
    }

    pub fn with_coinbase(mut self, maturity: u64, recovery_byte: u8) -> Self {
        self.features.flags |= OutputFlags::COINBASE_OUTPUT;
        self.features.maturity = maturity;
        self.features.recovery_byte = recovery_byte;
        self
    }

    pub fn with_recovery_byte(mut self, recovery_byte: u8) -> Self {
        self.features.recovery_byte = recovery_byte;
        self
    }

    pub fn add_flags(mut self, flags: OutputFlags) -> Self {
        self.features.flags.insert(flags);
        self
    }

    pub fn for_asset_registration(
        mut self,
        metadata: Vec<u8>,
        public_key: PublicKey,
        template_ids_implemented: Vec<u32>,
        template_parameters: Vec<TemplateParameter>,
    ) -> Self {
        self.add_flags(OutputFlags::ASSET_REGISTRATION);
        self
    }

    pub fn finish(self) -> OutputFeatures {
        self.features
    }
}
