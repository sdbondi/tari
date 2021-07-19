// Copyright 2020, The Tari Project
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
    schema::dedup_cache,
    storage::{DbConnection, StorageError},
};
use chrono::{NaiveDateTime, Utc};
use diesel::{dsl, result::DatabaseErrorKind, ExpressionMethods, QueryDsl, RunQueryDsl};
use log::*;
use std::cmp::max;
use tari_crypto::tari_utilities::ByteArray;
use tari_utilities::hex;

const LOG_TARGET: &str = "comms::dht::dedup_cache";

//#[derive(Clone, Debug, Queryable, Insertable, PartialEq)]
#[derive(Clone, Debug, Queryable, Identifiable)]
#[table_name = "dedup_cache"]
struct DedupCacheSql {
    pub id: i32,
    pub body_hash: String,
    pub number_of_hits: i32,
    pub stored_at: NaiveDateTime,
    pub last_hit_at: NaiveDateTime,
}

#[derive(Clone, Debug, Insertable, Default, AsChangeset)]
#[table_name = "dedup_cache"]
pub struct UpdateDedupCacheSql {
    pub body_hash: Option<String>,
    pub number_of_hits: Option<i32>,
    pub last_hit_at: Option<NaiveDateTime>,
}

#[derive(Clone)]
pub struct DedupCacheDatabase {
    connection: DbConnection,
    capacity: usize,
}

impl DedupCacheDatabase {
    pub fn new(connection: DbConnection, capacity: usize) -> Self {
        let capacity = max(capacity, 100);
        debug!(
            target: LOG_TARGET,
            "Message dedup cache capacity initialized at {}", capacity,
        );
        Self { connection, capacity }
    }

    /// Inserts and returns Ok(true) if the item already existed and Ok(false) if it didn't
    pub async fn insert_body_hash_if_unique(&self, body_hash: Vec<u8>) -> Result<bool, StorageError> {
        let body_hash_string = hex::to_hex(&body_hash.as_bytes());
        match self.insert_body_hash(body_hash_string.clone()).await {
            Ok(val) => {
                if val == 0 {
                    warn!(
                        target: LOG_TARGET,
                        "Unable to insert new entry into message dedup cache"
                    );
                }
                Ok(false)
            },
            Err(e) => match e {
                StorageError::UniqueViolation(_) => match self.update_number_of_hits(body_hash_string).await {
                    Ok(_) => Ok(true),
                    Err(e) => Err(e),
                },
                _ => Err(e),
            },
        }
    }

    /// Trims the dedup cache to the configured limit by removing the oldest entries
    pub async fn truncate(&self) -> Result<usize, StorageError> {
        let capacity = self.capacity;
        self.connection
            .with_connection_async(move |conn| {
                let mut num_removed = 0;
                let msg_count = dedup_cache::table
                    .select(dsl::count(dedup_cache::id))
                    .first::<i64>(conn)? as usize;
                // Hysteresis added to minimize database impact
                if msg_count > capacity {
                    let remove_count = msg_count - capacity;
                    let message_ids: Vec<i32> = dedup_cache::table
                        .select(dedup_cache::id)
                        .order_by(dedup_cache::stored_at.asc())
                        .limit(remove_count as i64)
                        .get_results(conn)?;
                    num_removed = diesel::delete(dedup_cache::table)
                        .filter(dedup_cache::id.eq_any(message_ids))
                        .execute(conn)?;
                }
                debug!(
                    target: LOG_TARGET,
                    "Message dedup cache: msg_count {}, capacity {}, num_removed {}", msg_count, capacity, num_removed,
                );
                Ok(num_removed)
            })
            .await
    }

    async fn insert_body_hash(&self, body_hash: String) -> Result<usize, StorageError> {
        self.connection
            .with_connection_async(move |conn| {
                let insert_result = diesel::insert_into(dedup_cache::table)
                    .values(UpdateDedupCacheSql {
                        body_hash: Some(body_hash.clone()),
                        number_of_hits: Some(1),
                        last_hit_at: Some(DedupCacheDatabase::formatted_naive_date_time()?),
                    })
                    .execute(conn)
                    .map_err(Into::into);
                match insert_result {
                    Ok(val) => Ok(val),
                    Err(diesel::result::Error::DatabaseError(kind, e_info)) => match kind {
                        DatabaseErrorKind::UniqueViolation => Err(StorageError::UniqueViolation(body_hash)),
                        _ => Err(diesel::result::Error::DatabaseError(kind, e_info).into()),
                    },
                    Err(e) => Err(e.into()),
                }
            })
            .await
    }

    async fn update_number_of_hits(&self, body_hash: String) -> Result<usize, StorageError> {
        let record_to_update = self.find_by_body_hash(body_hash.clone()).await?;
        self.connection
            .with_connection_async(move |conn| {
                diesel::update(dedup_cache::table.filter(dedup_cache::body_hash.eq(&body_hash)))
                    .set(UpdateDedupCacheSql {
                        body_hash: None,
                        number_of_hits: Some(record_to_update.number_of_hits + 1),
                        last_hit_at: Some(DedupCacheDatabase::formatted_naive_date_time()?),
                    })
                    .execute(conn)
                    .map_err(Into::into)
            })
            .await
    }

    async fn find_by_body_hash(&self, body_hash: String) -> Result<DedupCacheSql, StorageError> {
        self.connection
            .with_connection_async(move |conn| {
                Ok(dedup_cache::table
                    .filter(dedup_cache::body_hash.eq(body_hash))
                    .first::<DedupCacheSql>(conn)?)
            })
            .await
    }

    // This function makes it easier for humans to read the database entries; database field 'stored_at' has a
    // 'DEFAULT CURRENT_TIMESTAMP' setting and its format is '2021-07-23 04:01:14'. When the 'last_hit_at' field is
    // populated with 'Utc::now().naive_utc()' its format is '2021-07-23 04:01:14.235873992', which makes it difficult
    // to compare visually. Resolution less than one second is not required.
    fn formatted_naive_date_time() -> Result<NaiveDateTime, StorageError> {
        NaiveDateTime::parse_from_str(
            Utc::now().naive_utc().format("%Y-%m-%d %H:%M:%S").to_string().as_str(),
            "%Y-%m-%d %H:%M:%S",
        )
        .map_err(|e| StorageError::ParseError(e.to_string()))
    }
}
