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

use crate::metrics_server::error::MetricsServerError;
use log::*;
use opentelemetry::global;
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use std::{
    convert::Infallible,
    net::{SocketAddr, ToSocketAddrs},
};
use tokio::task;
use warp::{Filter, Rejection, Reply};

const LOG_TARGET: &str = "app::metrics_server";

pub async fn start<T: ToSocketAddrs>(address: T, exporter: PrometheusExporter) -> Result<(), MetricsServerError> {
    let bind_addr = address
        .to_socket_addrs()
        .map_err(|err| MetricsServerError::FailedToParseAddress(err.to_string()))?
        .next()
        .unwrap();

    let route = warp::path!("metrics")
        .and(with(exporter))
        .and_then(metrics_text_handler);

    info!(target: LOG_TARGET, "Metrics server started on {}", bind_addr);
    warp::serve(route.with(warp::log("metrics_server")))
        .run(bind_addr)
        .await;

    Ok(())
}

async fn metrics_text_handler(exporter: PrometheusExporter) -> Result<impl Reply, Rejection> {
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&exporter.registry().gather(), &mut buffer) {
        error!(target: LOG_TARGET, "could not encode prometheus metrics: {}", e);
        return Ok(e.to_string());
    };

    match String::from_utf8(buffer) {
        Ok(v) => return Ok(v),
        Err(e) => {
            error!(target: LOG_TARGET, "prometheus metrics were not valid UTF8: {}", e);
            Ok("prometheus metrics are not valid UTF8".to_string())
        },
    }
}

fn with<T: Clone + Send>(t: T) -> impl Filter<Extract = (T,), Error = Infallible> + Clone {
    warp::any().map(move || t.clone())
}
