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

#[cfg(test)]
mod test;

use crate::dan_layer::{
    models::TokenId,
    storage::{
        error::PersistenceError,
        traits::{AssetBackend, Atomic, AtomicAccess},
    },
};
use lmdb_zero as lmdb;
use lmdb_zero::{
    db,
    put,
    ConstAccessor,
    ConstTransaction,
    LmdbResultExt,
    ReadTransaction,
    WriteAccessor,
    WriteTransaction,
};
use std::{fs, fs::File, marker::PhantomData, path::Path, sync::Arc};
use tari_common::file_lock;
use tari_storage::lmdb_store::{DatabaseRef, LMDBBuilder, LMDBConfig, LMDBStore};

const DATABASES: &[(&str, db::Flags)] = &[("metadata", db::INTEGERKEY)];

#[derive(Clone)]
pub struct LmdbAssetBackend {
    _file_lock: Arc<File>,
    env: Arc<lmdb::Environment>,
    metadata_db: DatabaseRef,
}

impl LmdbAssetBackend {
    pub fn initialize<P: AsRef<Path>>(path: P, config: LMDBConfig) -> Result<Self, PersistenceError> {
        fs::create_dir_all(&path)?;
        let file_lock = file_lock::try_lock_exclusive(path.as_ref())?;
        let store = create_lmdb_store(path, config)?;

        Ok(Self {
            _file_lock: Arc::new(file_lock),
            env: store.env(),
            metadata_db: store.get_handle("metadata").unwrap().db(),
        })
    }
}

impl<'a> Atomic<'a> for LmdbAssetBackend {
    type Transaction = ConstTransaction<'a>;
    type WriteTransaction = WriteTransaction<'a>;

    fn acquire_read(&self) -> Result<Self::Transaction, PersistenceError> {
        let tx = ReadTransaction::new(*self.env)?;
        Ok(*tx)
    }

    fn acquire_write(&self) -> Result<Self::WriteTransaction, PersistenceError> {
        let tx = WriteTransaction::new(*self.env)?;
        Ok(tx)
    }
}

impl<'a> AssetBackend<'a> for LmdbAssetBackend {
    fn get_metadata(
        &self,
        txn: &'a <Self as Atomic>::Transaction,
        key: u64,
    ) -> Result<Option<&'a [u8]>, PersistenceError> {
        let val = txn.access().get::<_, [u8]>(&*self.metadata_db, &key).to_opt()?;
        Ok(val)
    }

    fn replace_metadata(
        &self,
        txn: &'a mut <Self as Atomic>::WriteTransaction,
        key: u64,
        metadata: &[u8],
    ) -> Result<(), PersistenceError> {
        let mut access = txn.access();
        access.put(&self.metadata_db, &key, metadata, put::Flags::empty())?;
        Ok(())
    }
}

impl<'a> AtomicAccess<'a> for &'a ConstTransaction<'a> {
    type Access = ConstAccessor<'a>;

    fn access(&'a mut self) -> Self::Access {
        ConstTransaction::access(self)
    }

    fn commit(self) -> Result<(), PersistenceError> {
        Ok(())
    }
}

impl<'a> AtomicAccess<'a> for &'a mut WriteTransaction<'a> {
    type Access = WriteAccessor<'a>;

    fn access(&'a mut self) -> Self::Access {
        WriteTransaction::access(&self.inner)
    }

    fn commit(self) -> Result<(), PersistenceError> {
        WriteTransaction::commit(self.inner).map_err(Into::into)
    }
}

fn create_lmdb_store<P: AsRef<Path>>(path: P, config: LMDBConfig) -> Result<LMDBStore, PersistenceError> {
    const CREATE_FLAG: db::Flags = db::CREATE;

    let mut lmdb_store = LMDBBuilder::new()
        .set_path(path)
        .set_env_config(config)
        .set_max_number_of_databases(10);

    for (db_name, flags) in DATABASES {
        lmdb_store = lmdb_store.add_database(db_name, CREATE_FLAG | *flags);
    }

    let lmdb_store = lmdb_store.build()?;
    Ok(lmdb_store)
}
