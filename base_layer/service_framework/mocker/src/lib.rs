use syn::export::TokenStream;

mod expand;

#[proc_macro_attribute]
pub fn service_mock(attr: TokenStream, item: TokenStream) -> TokenStream {
    let target_enum = syn::parse_macro_input!(item as syn::ItemEnum);
    let code = expand::expand(target_enum);
    let ts = quote::quote! { #code };
    ts.into()
}
