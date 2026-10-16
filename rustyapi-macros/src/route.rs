//! `#[get("/path", ...)]` и остальные макросы маршрутов.
//!
//! Функция превращается в unit-struct с тем же именем, реализующий `::rustyapi::routing::Endpoint`.
//! Исходная функция становится вложенной внутри `method_router()`.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Expr, ExprLit, FnArg, Ident, ItemFn, Lit, LitBool, LitStr, Meta, Token};

const ALLOWED: &str = "status_code, tags, summary, description, deprecated, include_in_schema";

struct RouteArgs {
    path: LitStr,
    status_code: Option<Expr>,
    tags: Vec<LitStr>,
    summary: Option<LitStr>,
    description: Option<LitStr>,
    deprecated: bool,
    include_in_schema: bool,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "не указан путь / missing route path\n\
                 подсказка / help: #[get(\"/items/{item_id}\")]",
            ));
        }
        let path: LitStr = input.parse().map_err(|err| {
            syn::Error::new(
                err.span(),
                "первым аргументом должен быть путь-строка / the first argument must be a path string\n\
                 подсказка / help: #[get(\"/items/{item_id}\")]",
            )
        })?;
        validate_path(&path)?;

        let mut args = RouteArgs {
            path,
            status_code: None,
            tags: Vec::new(),
            summary: None,
            description: None,
            deprecated: false,
            include_in_schema: true,
        };
        let mut seen: Vec<String> = Vec::new();
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let key = Ident::parse_any(input)?;
            let name = key.to_string();
            if seen.contains(&name) {
                return Err(syn::Error::new(
                    key.span(),
                    format!("параметр `{name}` указан дважды / duplicate parameter `{name}`"),
                ));
            }
            seen.push(name.clone());
            match name.as_str() {
                "status_code" => {
                    input.parse::<Token![=]>()?;
                    let expr: Expr = input.parse()?;
                    validate_status(&expr)?;
                    args.status_code = Some(expr);
                }
                "tags" => {
                    input.parse::<Token![=]>()?;
                    args.tags = parse_tags(input)?;
                }
                "summary" => {
                    input.parse::<Token![=]>()?;
                    args.summary = Some(parse_str(input, "summary")?);
                }
                "description" => {
                    input.parse::<Token![=]>()?;
                    args.description = Some(parse_str(input, "description")?);
                }
                "include_in_schema" => {
                    input.parse::<Token![=]>()?;
                    args.include_in_schema = input
                        .parse::<LitBool>()
                        .map_err(|err| {
                            syn::Error::new(
                                err.span(),
                                "`include_in_schema` принимает true/false / expects a bool",
                            )
                        })?
                        .value;
                }
                "deprecated" => {
                    args.deprecated = if input.peek(Token![=]) {
                        input.parse::<Token![=]>()?;
                        input
                            .parse::<LitBool>()
                            .map_err(|err| {
                                syn::Error::new(
                                    err.span(),
                                    "`deprecated` принимает true/false / expects a bool",
                                )
                            })?
                            .value
                    } else {
                        true
                    };
                }
                _ => {
                    let hint = if name == "status" || name == "code" {
                        "\nподсказка / help: используйте `status_code = 201`".to_owned()
                    } else if name == "tag" {
                        "\nподсказка / help: используйте `tags = [\"items\"]`".to_owned()
                    } else {
                        String::new()
                    };
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "неизвестный параметр `{name}` / unknown parameter `{name}`; \
                             допустимые / expected one of: {ALLOWED}{hint}"
                        ),
                    ));
                }
            }
        }
        Ok(args)
    }
}

fn parse_str(input: ParseStream<'_>, name: &str) -> syn::Result<LitStr> {
    input.parse::<LitStr>().map_err(|err| {
        syn::Error::new(
            err.span(),
            format!("`{name}` должен быть строкой / `{name}` must be a string literal"),
        )
    })
}

fn parse_tags(input: ParseStream<'_>) -> syn::Result<Vec<LitStr>> {
    if input.peek(LitStr) {
        return Ok(vec![input.parse()?]);
    }
    let err = |span| {
        syn::Error::new(
            span,
            "`tags` — строка или массив строк / `tags` must be a string or an array of strings\n\
             подсказка / help: tags = [\"items\"]",
        )
    };
    if !input.peek(syn::token::Bracket) {
        return Err(err(input.span()));
    }
    let content;
    syn::bracketed!(content in input);
    let items = content
        .parse_terminated(Expr::parse, Token![,])
        .map_err(|e| err(e.span()))?;
    items
        .into_iter()
        .map(|expr| match expr {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => Ok(s),
            other => Err(err(other.span())),
        })
        .collect()
}

fn validate_status(expr: &Expr) -> syn::Result<()> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Int(int), ..
    }) = expr
    {
        let ok = int
            .base10_parse::<u16>()
            .is_ok_and(|code| (100..=599).contains(&code));
        if !ok {
            return Err(syn::Error::new(
                int.span(),
                "status_code должен быть от 100 до 599 / status_code must be between 100 and 599",
            ));
        }
    }
    Ok(())
}

fn validate_path(lit: &LitStr) -> syn::Result<()> {
    let path = lit.value();
    let err = |msg: String| Err(syn::Error::new(lit.span(), msg));
    if !path.is_empty() && !path.starts_with('/') {
        return err(format!(
            "путь должен начинаться с '/' / route path must start with '/': {path:?}\n\
             подсказка / help: \"/{}\"",
            path.trim_start_matches('/')
        ));
    }
    let mut rest = path.as_str();
    let mut names: Vec<&str> = Vec::new();
    while let Some(start) = rest.find(['{', '}']) {
        if rest[start..].starts_with('}') {
            return err(format!(
                "лишняя '}}' в пути / unmatched '}}' in path {path:?}"
            ));
        }
        let Some(len) = rest[start..].find('}') else {
            return err(format!(
                "незакрытая '{{' в пути / unclosed '{{' in path {path:?}"
            ));
        };
        let param = &rest[start + 1..start + len];
        let (name, converter) = match param.split_once(':') {
            Some((name, converter)) => (name, Some(converter)),
            None => (param, None),
        };
        let valid_name = !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit());
        if !valid_name {
            return err(format!(
                "некорректное имя параметра пути {{{param}}} / invalid path parameter name\n\
                 подсказка / help: \"/items/{{item_id}}\""
            ));
        }
        if let Some(converter) = converter {
            if converter != "path" {
                return err(format!(
                    "конвертер `:{converter}` не поддерживается, только `:path` / only the `:path` converter is supported\n\
                     подсказка / help: тип задаётся экстрактором: Path<i64>"
                ));
            }
            if !rest[start + len + 1..].is_empty() {
                return err(format!(
                    "`{{{param}}}` должен быть в конце пути / a `:path` parameter must be last"
                ));
            }
        }
        if names.contains(&name) {
            return err(format!(
                "параметр `{name}` повторяется в пути / duplicate path parameter `{name}`"
            ));
        }
        names.push(name);
        rest = &rest[start + len + 1..];
    }
    Ok(())
}

pub fn expand(
    macro_name: &str,
    method: &str,
    args: TokenStream,
    item: TokenStream,
) -> syn::Result<TokenStream> {
    let args: RouteArgs = syn::parse2(args)?;
    let mut func: ItemFn = syn::parse2(item).map_err(|err| {
        syn::Error::new(
            err.span(),
            format!(
                "#[{macro_name}] применяется к `async fn` / #[{macro_name}] expects an `async fn`"
            ),
        )
    })?;

    if func.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            func.sig.fn_token.span(),
            format!(
                "обработчик должен быть `async fn` / route handler must be an `async fn`\n\
                 подсказка / help: `async fn {}(...)`",
                func.sig.ident
            ),
        ));
    }
    if !func.sig.generics.params.is_empty() || func.sig.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            func.sig.generics.span(),
            "обработчик не может быть generic / route handlers cannot be generic",
        ));
    }
    if let Some(FnArg::Receiver(receiver)) = func.sig.inputs.first() {
        return Err(syn::Error::new(
            receiver.span(),
            "обработчик не может принимать `self` — используйте свободную функцию / \
             route handlers cannot take `self`; use a free function",
        ));
    }

    let ident = func.sig.ident.clone();
    let name_str = ident.to_string();
    let vis = func.vis.clone();

    // doc-комментарии остаются на структуре и идут в description; остальные атрибуты — на функцию.
    let (doc_attrs, other_attrs): (Vec<_>, Vec<_>) = func
        .attrs
        .drain(..)
        .partition(|attr| attr.path().is_ident("doc"));
    let doc_text = doc_attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            Meta::NameValue(nv) => match &nv.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => Some(s.value()),
                _ => None,
            },
            _ => None,
        })
        .map(|line| line.strip_prefix(' ').map(str::to_owned).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    let doc_text = doc_text.trim();

    let method_ident = Ident::new(method, Span::call_site());
    let path = &args.path;

    let mut info = quote! {
        ::rustyapi::routing::RouteInfo::new(
            ::rustyapi::routing::__private::Method::#method_ident,
            #path,
            #name_str,
        )
    };
    if let Some(summary) = &args.summary {
        info = quote!(#info.summary(#summary));
    }
    if let Some(description) = &args.description {
        info = quote!(#info.description(#description));
    } else if !doc_text.is_empty() {
        info = quote!(#info.description(#doc_text));
    }
    if !args.tags.is_empty() {
        let tags = &args.tags;
        info = quote!(#info.tags([#(#tags),*]));
    }
    if args.deprecated {
        info = quote!(#info.deprecated(true));
    }
    if !args.include_in_schema {
        info = quote!(#info.include_in_schema(false));
    }

    // OpenAPI: аргументы и возвращаемый тип описывают себя через autoref-специализацию.
    let doc_args = func.sig.inputs.iter().filter_map(|arg| match arg {
        FnArg::Typed(pat) if !contains_impl_trait(&pat.ty) => {
            let ty = &pat.ty;
            Some(quote!((&&::rustyapi::openapi::__private::Probe::<#ty>::new()).__rustyapi_doc_arg(operation, components);))
        }
        _ => None,
    });
    let doc_response = match &func.sig.output {
        syn::ReturnType::Type(_, ty) if !contains_impl_trait(ty) => {
            quote!((&&::rustyapi::openapi::__private::Probe::<#ty>::new()).__rustyapi_doc_response(operation, components);)
        }
        syn::ReturnType::Type(..) => quote!(),
        syn::ReturnType::Default => {
            quote!((&&::rustyapi::openapi::__private::Probe::<()>::new()).__rustyapi_doc_response(operation, components);)
        }
    };
    info = quote! {
        #info.openapi({
            #[allow(unused_imports)]
            fn doc(
                operation: &mut ::rustyapi::openapi::__private::Operation,
                components: &mut ::rustyapi::openapi::__private::Components,
            ) {
                use ::rustyapi::openapi::__private::{NoArg as _, NoResponse as _, ViaArg as _, ViaResponse as _};
                #(#doc_args)*
                #doc_response
            }
            doc
        })
    };
    let status = match &args.status_code {
        Some(expr) => {
            info = quote!(#info.status_code(::rustyapi::routing::__private::status(#expr)));
            quote!(::core::option::Option::Some(::rustyapi::routing::__private::status(#expr)))
        }
        None => quote!(::core::option::Option::None),
    };

    // Понятная ошибка, если аргумент — не экстрактор (вместо «Handler is not implemented»).
    let arg_checks = func.sig.inputs.iter().filter_map(|arg| match arg {
        FnArg::Typed(pat) => {
            let ty = &pat.ty;
            Some(quote_spanned! {ty.span()=>
                const _: () = ::rustyapi::routing::__private::assert_extractor::<#ty, _>();
            })
        }
        FnArg::Receiver(_) => None,
    });

    let handler = format_ident!("__rustyapi_handler_{}", ident);
    func.sig.ident = handler.clone();
    func.vis = syn::Visibility::Inherited;

    Ok(quote! {
        #(#doc_attrs)*
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy)]
        #vis struct #ident;

        impl ::rustyapi::routing::Endpoint for #ident {
            fn info(&self) -> ::rustyapi::routing::RouteInfo {
                #info
            }

            fn method_router(&self) -> ::rustyapi::routing::__private::MethodRouter {
                #(#arg_checks)*
                #(#other_attrs)*
                #func

                ::rustyapi::routing::__private::on(
                    ::rustyapi::routing::__private::MethodFilter::#method_ident,
                    #handler,
                    #status,
                )
            }
        }
    })
}

/// Есть ли в типе `impl Trait` (такой тип нельзя подставить в `Probe::<T>`).
fn contains_impl_trait(ty: &syn::Type) -> bool {
    fn walk(tokens: TokenStream) -> bool {
        tokens.into_iter().any(|tt| match tt {
            proc_macro2::TokenTree::Ident(ident) => ident == "impl",
            proc_macro2::TokenTree::Group(group) => walk(group.stream()),
            _ => false,
        })
    }
    walk(quote::ToTokens::to_token_stream(ty))
}
