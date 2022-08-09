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

use std::marker::PhantomData;

use tari_template_abi::{Decode, Encode};

use crate::{hash::HashParseError, models::BucketId, Hash};

#[derive(Debug, Clone, Encode, Decode)]
pub enum ResourceRef<T> {
    Vault,
    VaultRef(ResourceAddress<T>),
    Bucket,
    BucketRef(BucketId),
}

#[derive(Debug)]
pub struct ResourceAddress<T> {
    address: Hash,
    _t: PhantomData<T>,
}

impl<T> ResourceAddress<T> {
    //     pub fn descriptor(&self) -> (Hash, UniversalTypeId) {
    //         (self.address, T::universal_type_id())
    //     }

    pub fn from_hex(s: &str) -> Result<Self, HashParseError> {
        Ok(ResourceAddress {
            address: Hash::from_hex(s)?,
            _t: PhantomData,
        })
    }
}

impl<T> Clone for ResourceAddress<T> {
    fn clone(&self) -> Self {
        Self {
            address: self.address,
            _t: PhantomData,
        }
    }
}

impl<T> Copy for ResourceAddress<T> {}
