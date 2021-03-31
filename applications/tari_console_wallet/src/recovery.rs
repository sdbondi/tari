// Copyright 2020. The Tari Project
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

use chrono::offset::Local;
use futures::{FutureExt, StreamExt};
use log::*;
use rustyline::Editor;
use tari_app_utilities::utilities::ExitCodes;
use tari_comms::peer_manager::Peer;
use tari_core::transactions::types::PrivateKey;
use tari_crypto::tari_utilities::hex::Hex;
use tari_key_manager::mnemonic::to_secretkey;
use tari_wallet::{
    tasks::wallet_recovery::{WalletRecoveryEvent, WalletRecoveryTask},
    WalletSqlite,
};

pub const LOG_TARGET: &str = "wallet::recovery";

/// Prompt the user to input their seed words in a single line.
pub fn prompt_private_key_from_seed_words() -> Result<PrivateKey, ExitCodes> {
    debug!(target: LOG_TARGET, "Prompting for seed words.");
    let mut rl = Editor::<()>::new();

    loop {
        println!("Recovery Mode");
        println!();
        println!("Type or paste all of your seed words on one line, only separated by spaces.");
        let input = rl.readline(">> ").map_err(|e| ExitCodes::IOError(e.to_string()))?;
        let seed_words: Vec<String> = input.split_whitespace().map(str::to_string).collect();

        match to_secretkey(&seed_words) {
            Ok(key) => break Ok(key),
            Err(e) => {
                debug!(target: LOG_TARGET, "MnemonicError parsing seed words: {}", e);
                println!("Failed to parse seed words! Did you type them correctly?");
                continue;
            },
        }
    }
}

/// Recovers wallet funds by connecting to a given base node peer, downloading the transaction outputs stored in the
/// blockchain, and attempting to rewind them. Any outputs that are successfully rewound are then imported into the
/// wallet.
pub async fn wallet_recovery(wallet: &mut WalletSqlite, base_node: &Peer) -> Result<(), ExitCodes> {
    let mut recovery_task = WalletRecoveryTask::new(wallet.clone(), base_node.public_key.clone());

    let mut event_stream = recovery_task
        .get_event_receiver()
        .ok_or_else(|| ExitCodes::RecoveryError("Unable to get recovery event stream".to_string()))?
        .fuse();

    let mut recovery_join_handle = tokio::spawn(recovery_task.run()).fuse();

    loop {
        futures::select! {
            event = event_stream.select_next_some() => {
                match event {
                    WalletRecoveryEvent::ConnectedToBaseNode(pk) => {
                        println!("Connected to base node: {}", pk.to_hex());
                    },
                    WalletRecoveryEvent::Progress(current, total) => {
                        let percentage_progress = ((current as f32) * 100f32 / (total as f32)).round() as u32;
                        println!("{}: Recovery process {}% complete.", Local::now(), percentage_progress);
                    },
                    WalletRecoveryEvent::Completed(num_utxos, total_amount) => {
                        println!("Recovered {} outputs with a value of {}", num_utxos, total_amount);
                    },
                }
            },
            recovery_result = recovery_join_handle => {
               return recovery_result.map_err(|e| ExitCodes::RecoveryError(format!("{}", e)))?.map_err(|e| ExitCodes::RecoveryError(format!("{}", e)));
            }
        }
    }
}
