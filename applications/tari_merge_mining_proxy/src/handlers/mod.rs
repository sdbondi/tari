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
mod tests;

mod get_block_template;
mod helpers;

use async_trait::async_trait;
use hyper::{Body, Request, Response};
use jsonrpc::serde_json::Value;
use serde_json as json;
use std::error;

pub trait ProxyHandlerPredicate {
    fn is_handler_for(&self, request: Request<json::Value>) -> bool;
}

pub trait ProxyHandlerJsonRpcPredicate: ProxyHandlerPredicate {
    const SUPPORTED_METHODS: &'static [&'static str];
}

impl<T: ProxyHandlerJsonRpcPredicate> ProxyHandlerPredicate for T {
    fn is_handler_for(&self, request: Request<Value>) -> bool {
        match request.body()["method"].as_str() {
            Some(method) => Self::SUPPORTED_METHODS.contains(&method),
            None => false,
        }
    }
}

#[async_trait]
pub trait ProxyHandler {
    type Error: error::Error;

    async fn handle(
        &self,
        request: Request<json::Value>,
        monero_resp: Response<json::Value>,
    ) -> Result<Response<Body>, Self::Error>;
}
