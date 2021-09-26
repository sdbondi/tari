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

use super::traits::Atomic;
use crate::{
    dan_layer::{
        models::TokenId,
        storage::{
            error::PersistenceError,
            lmdb::LmdbAssetBackend,
            traits::{AssetBackend, AtomicAccess},
        },
    },
    digital_assets_error::DigitalAssetError,
};
use bytecodec::{bincode_codec::BincodeDecoder, DecodeExt, EncodeExt, Error};
use lmdb_zero::ConstTransaction;
use patricia_tree::{
    node::{NodeDecoder, NodeEncoder},
    PatriciaMap,
};
use serde_json as json;
use std::str;

pub type LmdbAssetStore = AssetStore<LmdbAssetBackend>;

const PATRICIA_MAP_KEY: u64 = 1u64;

pub struct AssetStore<TBackend> {
    store: TBackend,
    map: Option<PatriciaMap<json::Value>>,
}

impl<'a, TBackend> AssetStore<TBackend>
where TBackend: AssetBackend<'a>
{
    pub fn new(store: TBackend) -> Self {
        Self { store, map: None }
    }

    /// Returns the full persisted ParticiaMap of the metadata state. This function is memoized so repeated calls will
    /// not result in a load.
    fn load_or_get_map(
        &mut self,
        txn: TBackend::Transaction,
    ) -> Result<&mut PatriciaMap<json::Value>, PersistenceError> {
        match self.map.as_mut() {
            None => {
                let map = self
                    .store
                    .get_metadata(txn, PATRICIA_MAP_KEY)?
                    .map(decode_patricia_map)
                    .transpose()?
                    .unwrap_or_default();
                self.map = Some(map);
                Ok(self.map.as_mut().unwrap())
            },
            Some(map) => Ok(map),
        }
    }
}

pub trait AssetDataStore<'a, TDb: Atomic<'a>> {
    fn read(&self) -> Result<TDb::Transaction, PersistenceError>;

    fn write(&mut self) -> Result<TDb::WriteTransaction, PersistenceError>;

    fn get_metadata(
        &mut self,
        txn: TDb::Transaction,
        token_id: &TokenId,
    ) -> Result<Option<&json::Value>, DigitalAssetError>;

    fn replace_metadata(
        &mut self,
        txn: TDb::WriteTransaction,
        token_id: &TokenId,
        metadata: &[u8],
    ) -> Result<(), DigitalAssetError>;
}

// TODO: Perhaps this belongs in a model
impl<'a, TBackend> AssetDataStore<'a, TBackend> for AssetStore<TBackend>
where TBackend: AssetBackend<'a>
{
    fn read(&self) -> Result<TBackend::Transaction, PersistenceError> {
        self.store.read()
    }

    fn write(&mut self) -> Result<TBackend::WriteTransaction, PersistenceError> {
        self.store.write()
    }

    fn get_metadata(
        &mut self,
        txn: TBackend::Transaction,
        token_id: &TokenId,
    ) -> Result<Option<&json::Value>, DigitalAssetError> {
        let map = self.load_or_get_map(&txn)?;
        let val = map.get(token_id);
        Ok(val)
    }

    fn replace_metadata(
        &mut self,
        txn: TBackend::WriteTransaction,
        token_id: &TokenId,
        metadata: &[u8],
    ) -> Result<(), DigitalAssetError> {
        let json = str::from_utf8(metadata)?;
        dbg!(&json);
        let map = self.load_or_get_map(&txn)?;
        let value = serde_json::from_str(&json)?;
        map.insert(token_id, value);
        let encoded = encode_patricia_map(map)?;
        self.store.replace_metadata(txn, PATRICIA_MAP_KEY, &encoded)?;
        txn.commit()?;
        Ok(())
    }
}

fn decode_patricia_map(bytes: &[u8]) -> Result<PatriciaMap<json::Value>, Error> {
    let mut decoder = NodeDecoder::new(BincodeDecoder::new());
    let node = decoder.decode_from_bytes(bytes)?;
    Ok(PatriciaMap::from(node))
}

fn encode_patricia_map(map: &PatriciaMap<json::Value>) -> Result<Vec<u8>, Error> {
    let mut encoder = NodeEncoder::new(BincodeDecoder::new());
    let encoded = encoder.encode_into_bytes(map.into())?;
    Ok(encoded)
}
