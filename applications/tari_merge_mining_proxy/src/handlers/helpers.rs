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
/// The JSON object key name used for merge mining proxy response extensions
pub(super) const MMPROXY_AUX_KEY_NAME: &str = "_aux";
/// The identifier used to identify the tari aux chain data
pub(super) const TARI_CHAIN_ID: &str = "xtr";

/// Add mmproxy extensions object to JSON RPC success response
pub fn add_aux_data(mut response: json::Value, mut ext: json::Value) -> json::Value {
    if response["result"].is_null() {
        return response;
    }
    match response["result"][MMPROXY_AUX_KEY_NAME].as_object_mut() {
        Some(obj_mut) => {
            let ext_mut = ext
                .as_object_mut()
                .expect("invalid parameter: expected `ext: json::Value` to be an object but it was not");
            obj_mut.append(ext_mut);
        },
        None => {
            response["result"][MMPROXY_AUX_KEY_NAME] = ext;
        },
    }
    response
}

pub fn append_aux_chain_data(mut response: json::Value, chain_data: json::Value) -> json::Value {
    if response["result"].is_null() {
        return response;
    }
    let chains = match response["result"][MMPROXY_AUX_KEY_NAME]["chains"].as_array_mut() {
        Some(arr_mut) => arr_mut,
        None => {
            response["result"][MMPROXY_AUX_KEY_NAME]["chains"] = json!([]);
            response["result"][MMPROXY_AUX_KEY_NAME]["chains"]
                .as_array_mut()
                .unwrap()
        },
    };

    chains.push(chain_data);
    response
}
