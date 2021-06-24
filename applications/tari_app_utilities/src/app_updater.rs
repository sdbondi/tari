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

use crate::consts as app_consts;
use futures::future;
use std::{env::consts, str::FromStr, sync::Arc};
use tari_common::configuration::bootstrap::ApplicationType;
use tari_p2p::{
    auto_update,
    auto_update::{AutoUpdateConfig, AutoUpdateError, SoftwareUpdate, Version},
};
use tari_service_framework::{ServiceInitializationError, ServiceInitializer, ServiceInitializerContext};
use tokio::sync::watch;

const LOG_TARGET: &str = "app:auto-update";

/// A watch notifier that contains the latest software update, if any
pub type SoftwareUpdateNotifier = watch::Receiver<Option<SoftwareUpdate>>;

#[derive(Clone)]
pub struct SoftwareUpdaterHandle {
    trigger: Arc<watch::Sender<()>>,
    notifier: SoftwareUpdateNotifier,
}

impl SoftwareUpdaterHandle {
    /// Returns watch notifier that triggers after a check for software updates
    pub fn notifier(&self) -> &SoftwareUpdateNotifier {
        &self.notifier
    }

    pub fn trigger_check_for_updates(&mut self) {
        let _ = self.trigger.broadcast(());
    }
}

pub struct SoftwareUpdater {
    application: ApplicationType,
    config: AutoUpdateConfig,
    is_enabled: bool,
}

impl SoftwareUpdater {
    pub fn new(application: ApplicationType, config: AutoUpdateConfig, is_enabled: bool) -> Self {
        Self {
            application,
            config,
            is_enabled,
        }
    }
}

impl ServiceInitializer for SoftwareUpdater {
    type Future = future::Ready<Result<(), ServiceInitializationError>>;

    fn initialize(&mut self, context: ServiceInitializerContext) -> Self::Future {
        if !self.is_enabled {
            return future::ready(Ok(()));
        }

        let config = self.config.clone();
        let app = self.application;

        let (notifier, notif_rx) = watch::channel(None);
        let (trigger_tx, mut trigger_rx) = watch::channel(());
        context.register_handle::<SoftwareUpdaterHandle>(SoftwareUpdaterHandle {
            notifier: notif_rx,
            trigger: Arc::new(trigger_tx),
        });

        context.spawn_until_shutdown(move |_| async move {
            loop {
                let _ = trigger_rx.recv().await;

                log::info!(
                    target: LOG_TARGET,
                    "Checking for updates ({})...",
                    config.update_uris.join(", ")
                );

                match check_for_updates(app, config.clone()).await {
                    Ok(maybe_update) => {
                        let _ = notifier.broadcast(maybe_update);
                    },
                    Err(err) => {
                        log::error!(target: LOG_TARGET, "Failed up fetch updates: {}", err);
                        let _ = notifier.broadcast(None);
                    },
                }
            }
        });

        future::ready(Ok(()))
    }
}

async fn check_for_updates(
    app: ApplicationType,
    config: AutoUpdateConfig,
) -> Result<Option<SoftwareUpdate>, AutoUpdateError> {
    let version = Version::from_str(app_consts::APP_VERSION_NUMBER)
        .expect("Unable to parse application version. Not valid semver");
    let arch = format!("{}-{}", consts::OS, consts::ARCH);

    match auto_update::check_for_updates(app, &arch, &version, config).await? {
        Some(update) => {
            log::info!(target: LOG_TARGET, "New update found {}", update);
            Ok(Some(update))
        },
        None => {
            log::info!(
                target: LOG_TARGET,
                "No new update found. Current: {} {} {}",
                app,
                version,
                arch
            );
            Ok(None)
        },
    }
}
