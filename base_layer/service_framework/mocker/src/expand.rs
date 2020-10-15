//  Copyright 2020, The Tari Project
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

use syn::{export::TokenStream2, fold, fold::Fold};

pub fn expand(item: syn::ItemEnum) -> TokenStream2 {
    let mut collector = RequestEnumInfoCollector::new();
    let enum_code = collector.fold_item_enum(node);
    // let generator = RpcCodeGenerator::new(options, collector.expect_trait_ident(), collector.rpc_methods);
    // let rpc_code = generator.generate();
    quote::quote! {
        #enum_code
    }
}

#[derive(Debug, Default)]
struct RequestEnumInfoCollector {
    enum_ident: Option<syn::Ident>,
    methods: Vec<ServiceMethodInfo>,
}

impl RequestEnumInfoCollector {
    pub fn new() -> Self {
        Default::default()
    }
}

impl Fold for RequestEnumInfoCollector {
    fn fold_item_enum(&mut self, node: syn::ItemEnum) -> syn::ItemEnum {
        self.trait_ident = Some(node.ident.clone());
        fold::fold_item_enum(self, node)
    }

    fn fold_variant(&mut self, node: syn::Variant) -> syn::Variant {
        self.methods.push((&node).into());
        fold::fold_variant(self, node)
    }
}

struct ServiceMethodInfo {
    name: syn::Ident,
    params: syn::Fields,
}

impl From<&syn::Variant> for ServiceMethodInfo {
    fn from(v: &syn::Variant) -> Self {
        Self {
            name: v.ident.clone(),
            params: v.fields.clone(),
        }
    }
}
