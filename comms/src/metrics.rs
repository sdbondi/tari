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

// use once_cell::sync::Lazy;
// use prometheus::{register_int_gauge_vec, IntGauge, IntGaugeVec};
//
// static CONNECTIONS: Lazy<IntGaugeVec> = Lazy::new(|| {
//     register_int_gauge_vec!("comms_connections", "Number of active connections by direction", &[
//         "role",
//         "node_id",
//         "public_key",
//         "direction",
//     ])
//     .unwrap()
// });
//

// pub fn connections(identity: &NodeIdentity, direction: ConnectionDirection) -> IntGauge {
//     CONNECTIONS.with_label_values(&[
//         to_role_str(identity.features()),
//         identity.node_id().to_string().as_str(),
//         identity.public_key().to_string().as_str(),
//         direction.as_str(),
//     ])
//
//     // let mut span = global_tracer().start("active_connections");
//     // span.set_attribute(KeyValue::new("role", to_role_str(identity.features())));
//     // span.set_attribute(KeyValue::new("node_id", identity.node_id().to_string()));
//     // span.set_attribute(KeyValue::new("public_key", identity.public_key().to_string()));
//     // span
// }

// #[cfg(feature = "rpc")]
// pub fn rpc() -> IntGauge {
//     todo!()
// }
//
// static ACTIVE_PROTOCOLS: Lazy<IntGaugeVec> = Lazy::new(|| {
//     register_int_gauge_vec!("active_rpc_sessions", "Number of active RPC sessions", &[
//         "role",
//         "node_id",
//         "public_key",
//         "protocol_name",
//     ])
//     .unwrap()
// });
//
// pub fn active_protocols(
//     node_id: &NodeId,
//     features: PeerFeatures,
//     public_key: &CommsPublicKey,
//     protocol_name: ProtocolId,
// ) -> IntGauge {
//     ACTIVE_PROTOCOLS.with_label_values(&[
//         node_id.to_string().as_str(),
//         to_role_str(features),
//         public_key.to_string().as_str(),
//         String::from_utf8_lossy(&protocol_name).as_ref(),
//     ])
// }

mod opentelemetry {
    use crate::{connection_manager::ConnectionDirection, peer_manager::PeerFeatures, NodeIdentity};
    use once_cell::sync::Lazy;
    use opentelemetry::{
        global,
        global::{BoxedSpan, BoxedTracer},
        metrics::{BoundValueRecorder, Meter, MeterProvider, Unit, ValueObserver, ValueRecorder},
        trace::{Span, Tracer},
        Key,
    };

    static GLOBAL_METER: Lazy<Meter> =
        Lazy::new(|| global::meter_provider().meter("tari-comms", Some(env!("CARGO_PKG_VERSION"))));

    static CONNECTIONS: Lazy<ValueRecorder<i64>> = Lazy::new(|| GLOBAL_METER.i64_value_recorder("connections").init());

    pub fn record_connections(identity: &NodeIdentity, direction: ConnectionDirection, value: i64) {
        CONNECTIONS.record(value, &[
            Key::new("role").string(identity.features().as_role_str()),
            Key::new("node_id").string(identity.node_id().to_string()),
            Key::new("public_key").string(identity.public_key().to_string()),
            Key::new("direction").string(direction.as_str()),
        ])
    }
}
pub use self::opentelemetry::*;
