//! `#[rustyapi::main]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{ItemFn, ReturnType};

pub fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !args.is_empty() {
        return Err(syn::Error::new(
            args.span(),
            "#[rustyapi::main] не принимает аргументов / takes no arguments",
        ));
    }
    let mut func: ItemFn = syn::parse2(item)?;
    if func.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            func.sig.fn_token.span(),
            "#[rustyapi::main] применяется к `async fn` / expected `async fn main`\n\
             подсказка / help: напишите `async fn main() -> rustyapi::Result<()>`",
        ));
    }
    if !func.sig.inputs.is_empty() {
        return Err(syn::Error::new(
            func.sig.inputs.span(),
            "у `main` не может быть аргументов / `main` cannot take arguments",
        ));
    }
    if !func.sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            func.sig.generics.span(),
            "у `main` не может быть generic-параметров / `main` cannot be generic",
        ));
    }

    func.sig.asyncness = None;
    let output = match &func.sig.output {
        ReturnType::Default => quote!(()),
        ReturnType::Type(_, ty) => quote!(#ty),
    };
    let attrs = &func.attrs;
    let vis = &func.vis;
    let sig = &func.sig;
    let body = &func.block;
    Ok(quote! {
        #(#attrs)*
        #vis #sig {
            ::rustyapi::__private::block_on::<#output, _>(async move #body)
        }
    })
}

/// Ошибка + пустой `fn main() {}`, чтобы не было лишней ошибки «`main` function not found».
pub fn with_stub(err: syn::Error, item: TokenStream) -> TokenStream {
    let error = err.into_compile_error();
    let stub = syn::parse2::<ItemFn>(item).ok().map(|func| {
        let vis = &func.vis;
        let ident = &func.sig.ident;
        quote!(#vis fn #ident() {})
    });
    quote!(#error #stub)
}
