mod client;
pub use client::DnsClient;

mod error;
pub use error::DnsClientError;

#[cfg(test)]
pub(crate) mod mock;
