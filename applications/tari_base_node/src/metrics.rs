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

use crate::metrics_server;
use futures::Stream;
use opentelemetry::{
    global,
    sdk::{metrics::PushController, Resource},
    KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use std::{env, process, time::Duration};
use tari_app_utilities::consts;
use tokio::{task, time};
use tokio_stream::wrappers::IntervalStream;
use tonic::metadata::{MetadataMap, MetadataValue};

pub fn enable() -> () {
    // let mut metadata = MetadataMap::new();
    // metadata.insert("app_name", MetadataValue::from_static("tari-base-node"));
    // metadata.insert("app_version", MetadataValue::from_static(consts::APP_VERSION));
    // let exporter = opentelemetry_otlp::new_exporter()
    //     .tonic()
    //     .with_metadata(metadata)
    //     .with_endpoint("http://localhost:4317");

    let prometheus = opentelemetry_prometheus::exporter()
        .with_resource(Resource::new([
            KeyValue::new("pid", process::id().to_string()),
            KeyValue::new("version", consts::APP_VERSION),
        ]))
        .init();

    task::spawn(metrics_server::start(("0.0.0.0", 5544), prometheus));
    // let controller = opentelemetry_otlp::new_pipeline()
    //     .metrics(tokio::spawn, tokio_interval_to_futures_stream)
    //     .with_exporter(exporter)
    //     .with_period(Duration::from_secs(5))
    //     .build()
    //     .unwrap();
    // global::set_meter_provider(controller.provider());
    // controller
}

fn tokio_interval_to_futures_stream(duration: Duration) -> impl Stream<Item = time::Instant> {
    IntervalStream::new(time::interval(duration))
}
