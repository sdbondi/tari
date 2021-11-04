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
use crate::dns::mock::MockClientHandle;

use super::DnsClientError;
use trust_dns_resolver::{
    config::{ResolverConfig, ResolverOpts},
    AsyncResolver,
    IntoName,
    TokioAsyncResolver,
};

#[derive(Clone)]
pub enum DnsClient {
    Resolver(TokioAsyncResolver),
    #[cfg(test)]
    Mock(MockClientHandle),
}

impl DnsClient {
    pub async fn connect_secure() -> Result<Self, DnsClientError> {
        let options = ResolverOpts {
            // Enable DNSSec validation
            validate: true,
            ..Default::default()
        };
        let resolver = AsyncResolver::tokio(ResolverConfig::cloudflare_tls(), options)?;
        Ok(DnsClient::Resolver(resolver))
    }

    pub async fn connect() -> Result<Self, DnsClientError> {
        let options = ResolverOpts {
            validate: false,
            ..Default::default()
        };
        let resolver = AsyncResolver::tokio(ResolverConfig::cloudflare_tls(), options)?;
        Ok(DnsClient::Resolver(resolver))
    }

    #[cfg(test)]
    pub async fn connect_mock(messages: Vec<String>) -> Result<Self, DnsClientError> {
        let client = MockClientHandle::new(messages);
        Ok(DnsClient::Mock(client))
    }

    pub async fn lookup_txt<T: IntoName>(&mut self, name: T) -> Result<Vec<String>, DnsClientError> {
        use DnsClient::*;
        let response = match self {
            Resolver(client) => client.txt_lookup(name).await?,
            #[cfg(test)]
            Mock(client) => {
                return Ok(client.messages().to_vec());
            },
        };

        let records = response
            .iter()
            .filter_map(|txt| {
                let data = txt.txt_data();
                if data.is_empty() {
                    return None;
                }
                // Exclude the first length octet from the string result
                Some(data.iter().map(|bytes| String::from_utf8_lossy(bytes).to_string()))
            })
            .flatten()
            .collect();

        Ok(records)
    }
}
