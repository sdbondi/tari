//  Copyright 2021, The Tari Project
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

use crate::dan_layer::{
    models::TokenId,
    storage::{
        error::PersistenceError,
        traits::{Atomic, AtomicAccess},
        AssetBackend,
    },
};
use patricia_tree::PatriciaMap;
use serde_json as json;
use std::{
    marker::PhantomData,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

pub struct MemoryAssetBackend {
    inner: RwLock<MemoryAssetStore>,
}

pub struct MemoryAssetStore {
    metadata: Vec<Vec<u8>>,
}

pub struct RwLockReadAccess<'a, T> {
    inner: RwLockReadGuard<'a, T>,
}
// impl<'a, T> AtomicAccess for RwLockReadGuard<'a, T> {
//     type Access = RwLockReadAccess<'a, T>;
//     type Error = ();
//
//     fn access(&self) -> Self::Access {
//         RwLockReadAccess { inner: self }
//     }
//
//     fn commit(self) -> Result<(), Self::Error> {
//         Ok(())
//     }
// }

impl<'a> Atomic<'a> for MemoryAssetBackend {
    type Transaction = RwLockReadGuard<'a, MemoryAssetStore>;
    type WriteTransaction = RwLockWriteGuard<'a, MemoryAssetStore>;

    fn acquire_read(&self) -> Result<Self::Transaction, PersistenceError> {
        Ok(self.inner.read()?)
    }

    fn acquire_write(&self) -> Result<Self::WriteTransaction, PersistenceError> {
        Ok(self.inner.write()?)
    }
}

impl<'a> AssetBackend<'a> for MemoryAssetBackend {
    fn get_metadata(
        &self,
        txn: &<Self as Atomic>::Transaction,
        index: u64,
    ) -> Result<Option<&'a [u8]>, PersistenceError> {
        Ok(Some(&txn.metadata[index as usize]))
    }

    fn replace_metadata(
        &self,
        txn: &'a mut <Self as Atomic>::WriteTransaction,
        index: u64,
        metadata: &[u8],
    ) -> Result<(), PersistenceError> {
        txn.metadata.insert(index as usize, metadata.to_vec());
        Ok(())
    }
}

impl<'a> AtomicAccess<'a> for RwLockReadGuard<'a, MemoryAssetStore> {
    type Access = &'a MemoryAssetStore;

    fn access(&'a mut self) -> Self::Access {
        self
    }

    fn commit(self) -> Result<(), PersistenceError> {
        Ok(())
    }
}

impl<'a> AtomicAccess<'a> for RwLockWriteGuard<'a, MemoryAssetStore> {
    type Access = &'a mut MemoryAssetStore;

    fn access(&mut self) -> Self::Access {
        self
    }

    // TODO: implement commit/rollback
    fn commit(self) -> Result<(), PersistenceError> {
        Ok(())
    }
}
