use crate::dummy_data::get_dummy_contacts;
use log::*;
use qrcode::{render::unicode, QrCode};
use std::sync::Arc;
use tari_common::Network;
use tari_comms::NodeIdentity;
use tari_crypto::tari_utilities::hex::Hex;
use tari_wallet::{
    contacts_service::storage::{database::Contact, sqlite_db::ContactsServiceSqliteDatabase},
    output_manager_service::storage::sqlite_db::OutputManagerSqliteDatabase,
    storage::sqlite_db::WalletSqliteDatabase,
    transaction_service::storage::{database::CompletedTransaction, sqlite_db::TransactionServiceSqliteDatabase},
    util::emoji::EmojiId,
    Wallet,
};

#[derive(Clone)]
pub struct UiContact {
    pub alias: String,
    pub public_key: String,
    pub emoji_id: String,
}

impl From<Contact> for UiContact {
    fn from(c: Contact) -> Self {
        Self {
            alias: c.alias,
            public_key: format!("{}", c.public_key),
            emoji_id: EmojiId::from_pubkey(&c.public_key).as_str().to_string(),
        }
    }
}
