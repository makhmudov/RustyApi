//! `#[model(table = "...")]` = `#[schema]` + `FromRow` (свой impl через `::rustyapi::sqlx`) + `Model`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Ident, LitStr, Token};

use crate::schema;

struct ModelArgs {
    table: LitStr,
}

impl Parse for ModelArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let hint = "подсказка / help: #[model(table = \"todos\")]";
        if input.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("не указана таблица / missing table name\n{hint}"),
            ));
        }
        let mut table = None;
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            if key != "table" {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "неизвестный параметр `{key}` / unknown parameter; допустимые / expected: table\n{hint}"
                    ),
                ));
            }
            input.parse::<Token![=]>()?;
            let value: LitStr = input.parse().map_err(|e| {
                syn::Error::new(
                    e.span(),
                    format!("`table` — строка / `table` must be a string\n{hint}"),
                )
            })?;
            if value.value().is_empty() {
                return Err(syn::Error::new(
                    value.span(),
                    "пустое имя таблицы / empty table name",
                ));
            }
            table = Some(value);
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(Self {
            table: table.expect("checked above"),
        })
    }
}

pub fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args: ModelArgs = syn::parse2(args)?;
    let mut input: DeriveInput = syn::parse2(item)?;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "#[model] не может быть generic / #[model] cannot be generic",
        ));
    }

    // `#[primary_key]` — наш атрибут: убираем до derive.
    let mut pk: Option<(Ident, syn::Type)> = None;
    let Data::Struct(data) = &mut input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "#[model] применяется к структуре с именованными полями / #[model] expects a struct",
        ));
    };
    let Fields::Named(named) = &mut data.fields else {
        return Err(syn::Error::new(
            data.fields.span(),
            "#[model] поддерживает только именованные поля / #[model] supports only named fields",
        ));
    };
    for field in &mut named.named {
        let before = field.attrs.len();
        field.attrs.retain(|a| !a.path().is_ident("primary_key"));
        if field.attrs.len() != before {
            if let Some((other, _)) = &pk {
                return Err(syn::Error::new(
                    field.span(),
                    format!("первичный ключ уже задан полем `{other}` / duplicate #[primary_key]"),
                ));
            }
            pk = Some((field.ident.clone().expect("named"), field.ty.clone()));
        }
    }
    if pk.is_none() {
        pk = named
            .named
            .iter()
            .find(|f| f.ident.as_ref().is_some_and(|i| i == "id"))
            .map(|f| (f.ident.clone().expect("named"), f.ty.clone()));
    }
    let Some((pk_ident, pk_ty)) = pk else {
        return Err(syn::Error::new(
            input.ident.span(),
            "у модели нет первичного ключа / model has no primary key\n\
             подсказка / help: добавьте поле `id` или пометьте поле `#[primary_key]`",
        ));
    };

    let ident = input.ident.clone();
    let name = ident.to_string();
    let table = &args.table;
    let expanded = schema::expand_input(input, &schema::Options::default())?;
    let schema_tokens = expanded.tokens;

    let columns: Vec<String> = expanded.fields.iter().map(|f| raw(&f.ident)).collect();
    let getters = expanded.fields.iter().map(|f| {
        let ident = &f.ident;
        let column = raw(ident);
        quote! {
            #ident: ::rustyapi::db::__private::Row::try_get(row, #column)?
        }
    });
    let pk_name = raw(&pk_ident);

    Ok(quote! {
        #schema_tokens

        impl<'__r> ::rustyapi::db::__private::FromRow<'__r, ::rustyapi::db::__private::PgRow> for #ident {
            fn from_row(
                row: &'__r ::rustyapi::db::__private::PgRow,
            ) -> ::core::result::Result<Self, ::rustyapi::db::__private::Error> {
                ::core::result::Result::Ok(Self {
                    #(#getters,)*
                })
            }
        }

        impl ::rustyapi::db::Model for #ident {
            const NAME: &'static str = #name;
            const TABLE: &'static str = #table;
            const PK: &'static str = #pk_name;
            const COLUMNS: &'static [&'static str] = &[#(#columns),*];
            type Id = #pk_ty;
        }
    })
}

fn raw(ident: &Ident) -> String {
    let name = ident.to_string();
    name.strip_prefix("r#").map_or(name.clone(), str::to_owned)
}
