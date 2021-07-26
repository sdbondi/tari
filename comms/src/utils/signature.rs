// Copyright 2019, The Tari Project
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

use crate::types::{Challenge, CommsPublicKey};
use blake2::digest::FixedOutput;
use digest::Digest;
use rand::{CryptoRng, Rng};
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    signatures::{SchnorrSignature, SchnorrSignatureError},
    tari_utilities::message_format::MessageFormat,
};

pub fn sign<R, B>(
    rng: &mut R,
    secret_key: <CommsPublicKey as PublicKey>::K,
    body: B,
) -> Result<SchnorrSignature<CommsPublicKey, <CommsPublicKey as PublicKey>::K>, SchnorrSignatureError>
where
    R: CryptoRng + Rng,
    B: AsRef<[u8]>,
{
    let challenge = Challenge::new().chain(body).finalize_fixed();
    let nonce = <CommsPublicKey as PublicKey>::K::random(rng);
    SchnorrSignature::sign(secret_key, nonce, challenge.as_slice())
}

/// Verify that the signature is valid for the message body
pub fn verify<B>(public_key: &CommsPublicKey, signature: &[u8], body: B) -> bool
where B: AsRef<[u8]> {
    match SchnorrSignature::<CommsPublicKey, <CommsPublicKey as PublicKey>::K>::from_binary(signature) {
        Ok(signature) => {
            let challenge = Challenge::new().chain(body).finalize_fixed();
            signature.verify_challenge(public_key, challenge.as_slice())
        },
        Err(_) => false,
    }
}
