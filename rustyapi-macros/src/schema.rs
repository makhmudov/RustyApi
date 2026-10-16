//! `#[schema]`: аналог `pydantic.BaseModel`.
//!
//! Добавляет `Serialize`/`Deserialize` (через `::rustyapi::serde`), `Debug`, `Clone` (если их нет),
//! реализует `Validate` (правила `#[field(...)]`) и `ApiSchema` (JSON Schema для OpenAPI).

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, ExprUnary, Fields, GenericParam, Lit, LitStr,
    Meta, Token, Type, UnOp,
};

const FIELD_ALLOWED: &str = "min_length, max_length, gt, ge, lt, le, pattern, email, url, \
                             default, example, title, description";

/// Опции для переиспользования из `#[model]`.
#[derive(Default)]
pub struct Options {
    /// Дополнительные derive (полные пути).
    pub extra_derives: Vec<TokenStream>,
}

pub fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !args.is_empty() {
        return Err(syn::Error::new(
            args.span(),
            "#[schema] не принимает аргументов / #[schema] takes no arguments",
        ));
    }
    let input: DeriveInput = syn::parse2(item)?;
    expand_input(input, &Options::default()).map(|e| e.tokens)
}

/// Результат раскрытия: код и сведения о полях (нужны `#[model]`, этап 4).
#[allow(dead_code)]
pub struct Expanded {
    pub tokens: TokenStream,
    pub fields: Vec<FieldInfo>,
}

/// Поле структуры после разбора атрибутов.
#[allow(dead_code)]
pub struct FieldInfo {
    pub ident: syn::Ident,
    pub ty: Type,
    pub wire_name: String,
    pub skip: bool,
    /// Атрибуты, оставшиеся на поле после разбора (`#[field]` удалён).
    pub attrs: Vec<Attribute>,
}

pub fn expand_input(mut input: DeriveInput, options: &Options) -> syn::Result<Expanded> {
    let container = ContainerSerde::from_attrs(&input.attrs)?;
    let derives = missing_derives(&input.attrs, options);
    let has_serde_crate = has_serde_crate(&input.attrs);
    let doc = doc_text(&input.attrs);
    let ident = input.ident.clone();

    let (validate_body, schema_body, fields_info) = match &mut input.data {
        Data::Struct(data) => {
            let Fields::Named(named) = &mut data.fields else {
                return Err(syn::Error::new(
                    data.fields.span(),
                    "#[schema] поддерживает только структуры с именованными полями / \
                     #[schema] supports only structs with named fields\n\
                     подсказка / help: struct Item { name: String }",
                ));
            };
            let mut validate = Vec::new();
            let mut props = Vec::new();
            let mut defaults = Vec::new();
            let mut infos = Vec::new();
            for field in &mut named.named {
                let rules = FieldRules::take_from(&mut field.attrs)?;
                let serde = FieldSerde::from_attrs(&field.attrs)?;
                let field_ident = field.ident.clone().expect("named field");
                let wire = serde
                    .rename
                    .clone()
                    .unwrap_or_else(|| container.rename_field(&raw_name(&field_ident)));
                let field_doc = doc_text(&field.attrs);

                // `#[field(default = ...)]` → `#[serde(default = "Type::__rustyapi_default_x")]`.
                let default_fn = if let Some(expr) = &rules.default {
                    let fn_ident = format_ident!("__rustyapi_default_{}", field_ident);
                    let ty = &field.ty;
                    let value = default_value(expr, option_inner(ty).is_some());
                    defaults.push(quote! {
                        #[doc(hidden)]
                        #[allow(clippy::useless_conversion)]
                        fn #fn_ident() -> #ty {
                            #value
                        }
                    });
                    let path = LitStr::new(&format!("{ident}::{fn_ident}"), Span::call_site());
                    field
                        .attrs
                        .push(syn::parse_quote!(#[serde(default = #path)]));
                    Some(fn_ident)
                } else {
                    None
                };

                infos.push(FieldInfo {
                    ident: field_ident.clone(),
                    ty: field.ty.clone(),
                    wire_name: wire.clone(),
                    skip: serde.skip,
                    attrs: field.attrs.clone(),
                });
                if serde.skip {
                    continue;
                }
                validate.push(field_validate(&field_ident, &field.ty, &wire, &rules));
                props.push(field_schema(
                    &field.ty,
                    &field_ident,
                    &wire,
                    &rules,
                    &field_doc,
                    default_fn.as_ref(),
                    serde.has_default || container.default || serde.skip_deserializing,
                ));
            }
            let title = ident.to_string();
            let description = doc.as_ref().map(
                |d| quote!(schema.insert("description".into(), ::rustyapi::serde_json::json!(#d));),
            );
            let defaults_impl = if defaults.is_empty() {
                quote!()
            } else {
                let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
                quote! {
                    impl #impl_generics #ident #ty_generics #where_clause {
                        #(#defaults)*
                    }
                }
            };
            let schema = quote! {
                let mut properties = ::rustyapi::serde_json::Map::new();
                let mut required: ::std::vec::Vec<::rustyapi::serde_json::Value> = ::std::vec::Vec::new();
                #(#props)*
                let mut schema = ::rustyapi::serde_json::Map::new();
                schema.insert("properties".into(), ::rustyapi::serde_json::Value::Object(properties));
                schema.insert("type".into(), ::rustyapi::serde_json::json!("object"));
                if !required.is_empty() {
                    schema.insert("required".into(), ::rustyapi::serde_json::Value::Array(required));
                }
                schema.insert("title".into(), ::rustyapi::serde_json::json!(#title));
                #description
                ::rustyapi::serde_json::Value::Object(schema)
            };
            (quote!(#(#validate)*), (schema, defaults_impl, title), infos)
        }
        Data::Enum(data) => {
            let mut names = Vec::new();
            for variant in &data.variants {
                if !matches!(variant.fields, Fields::Unit) {
                    return Err(syn::Error::new(
                        variant.span(),
                        "#[schema] поддерживает только enum из unit-вариантов (строковый enum) / \
                         #[schema] supports only enums with unit variants (string enums)\n\
                         подсказка / help: enum Status { Todo, Done }",
                    ));
                }
                let serde = FieldSerde::from_attrs(&variant.attrs)?;
                if serde.skip {
                    continue;
                }
                names.push(
                    serde
                        .rename
                        .unwrap_or_else(|| container.rename_variant(&variant.ident.to_string())),
                );
            }
            let title = ident.to_string();
            let description = doc.as_ref().map(
                |d| quote!(schema.insert("description".into(), ::rustyapi::serde_json::json!(#d));),
            );
            let schema = quote! {
                let mut schema = ::rustyapi::serde_json::Map::new();
                schema.insert("type".into(), ::rustyapi::serde_json::json!("string"));
                schema.insert("enum".into(), ::rustyapi::serde_json::json!([#(#names),*]));
                schema.insert("title".into(), ::rustyapi::serde_json::json!(#title));
                #description
                ::rustyapi::serde_json::Value::Object(schema)
            };
            (quote!(), (schema, quote!(), title), Vec::new())
        }
        Data::Union(data) => {
            return Err(syn::Error::new(
                data.union_token.span(),
                "#[schema] не поддерживает union / #[schema] does not support unions",
            ));
        }
    };
    let (schema_body, defaults_impl, title) = schema_body;

    // Имя схемы: `Page` или для generic `Page_Item_` (как Pydantic v2).
    let type_params: Vec<_> = input
        .generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some(t.ident.clone()),
            _ => None,
        })
        .collect();
    let schema_name = if type_params.is_empty() {
        quote!(::std::string::String::from(#title))
    } else {
        quote! {{
            let mut name = ::std::string::String::from(#title);
            #(
                name.push('_');
                name.push_str(&<#type_params as ::rustyapi::schema::ApiSchema>::schema_name()
                    .unwrap_or_else(|| ::std::string::String::from(stringify!(#type_params))));
            )*
            name.push('_');
            name
        }}
    };

    let mut validate_generics = input.generics.clone();
    let mut schema_generics = input.generics.clone();
    for param in &type_params {
        validate_generics
            .make_where_clause()
            .predicates
            .push(syn::parse_quote!(#param: ::rustyapi::schema::Validate));
        schema_generics
            .make_where_clause()
            .predicates
            .push(syn::parse_quote!(#param: ::rustyapi::schema::ApiSchema));
    }
    let (_, ty_generics, _) = input.generics.split_for_impl();
    let (v_impl, _, v_where) = validate_generics.split_for_impl();
    let (s_impl, _, s_where) = schema_generics.split_for_impl();

    let serde_crate = if has_serde_crate {
        quote!()
    } else {
        quote!(#[serde(crate = "::rustyapi::serde")])
    };
    let derive_attr = if derives.is_empty() {
        quote!()
    } else {
        quote!(#[derive(#(#derives),*)])
    };

    // Структуры принимают только JSON-объект (как Pydantic): serde по умолчанию принял бы и массив.
    // Derive генерирует inherent `Self::deserialize` (`remote = "Self"`), а трейты — обёртки над ним.
    let is_struct = matches!(input.data, Data::Struct(_));
    let (remote_attr, serde_impls) = if is_struct {
        let mut ser_generics = input.generics.clone();
        let mut de_generics = input.generics.clone();
        for param in &type_params {
            ser_generics
                .make_where_clause()
                .predicates
                .push(syn::parse_quote!(#param: ::rustyapi::serde::Serialize));
            de_generics
                .make_where_clause()
                .predicates
                .push(syn::parse_quote!(#param: ::rustyapi::serde::de::DeserializeOwned));
        }
        let (ser_impl, _, ser_where) = ser_generics.split_for_impl();
        de_generics
            .params
            .insert(0, syn::parse_quote!('__rustyapi_de));
        let (de_impl, _, de_where) = de_generics.split_for_impl();
        let visitor_generics = {
            let mut g = input.generics.clone();
            g.where_clause = None;
            for param in &mut g.params {
                if let GenericParam::Type(t) = param {
                    t.default = None;
                }
            }
            g
        };
        let expecting = format!("struct {ident}");
        (
            quote!(#[serde(remote = "Self")]),
            quote! {
                impl #ser_impl ::rustyapi::serde::Serialize for #ident #ty_generics #ser_where {
                    fn serialize<__S>(&self, serializer: __S) -> ::core::result::Result<__S::Ok, __S::Error>
                    where
                        __S: ::rustyapi::serde::Serializer,
                    {
                        Self::serialize(self, serializer)
                    }
                }

                impl #de_impl ::rustyapi::serde::Deserialize<'__rustyapi_de> for #ident #ty_generics #de_where {
                    fn deserialize<__D>(deserializer: __D) -> ::core::result::Result<Self, __D::Error>
                    where
                        __D: ::rustyapi::serde::Deserializer<'__rustyapi_de>,
                    {
                        struct __Visitor #visitor_generics (::core::marker::PhantomData<fn() -> #ident #ty_generics>);

                        impl #de_impl ::rustyapi::serde::de::Visitor<'__rustyapi_de> for __Visitor #ty_generics #de_where {
                            type Value = #ident #ty_generics;

                            fn expecting(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                                f.write_str(#expecting)
                            }

                            fn visit_map<__A>(self, map: __A) -> ::core::result::Result<Self::Value, __A::Error>
                            where
                                __A: ::rustyapi::serde::de::MapAccess<'__rustyapi_de>,
                            {
                                <#ident #ty_generics>::deserialize(
                                    ::rustyapi::serde::de::value::MapAccessDeserializer::new(map),
                                )
                            }
                        }

                        deserializer.deserialize_map(__Visitor(::core::marker::PhantomData))
                    }
                }
            },
        )
    } else {
        (quote!(), quote!())
    };

    let tokens = quote! {
        #derive_attr
        #serde_crate
        #remote_attr
        #input

        #serde_impls

        #defaults_impl

        impl #v_impl ::rustyapi::schema::Validate for #ident #ty_generics #v_where {
            #[allow(unused_variables, unused_imports)]
            fn validate_into(
                &self,
                loc: &mut ::rustyapi::schema::Loc,
                errors: &mut ::std::vec::Vec<::rustyapi::schema::ErrorDetail>,
            ) {
                use ::rustyapi::schema::__private::{NoValidate as _, ViaValidate as _};
                #validate_body
            }
        }

        impl #s_impl ::rustyapi::schema::ApiSchema for #ident #ty_generics #s_where {
            fn schema_name() -> ::core::option::Option<::std::string::String> {
                ::core::option::Option::Some(#schema_name)
            }

            #[allow(unused_imports, unused_mut)]
            fn schema(
                components: &mut ::rustyapi::schema::Components,
            ) -> ::rustyapi::serde_json::Value {
                use ::rustyapi::schema::__private::{
                    NoApiSchema as _, NoNumeric as _, ViaApiSchema as _, ViaNumeric as _,
                };
                let name = #schema_name;
                components.register(&name, |components| { #schema_body })
            }
        }
    };
    Ok(Expanded {
        tokens,
        fields: fields_info,
    })
}

/// Значение `default = ...`: числа и bool — как есть (тип выводится из поля), строки — через
/// `Into` (`"abc"` → `String`), для `Option`-полей литерал оборачивается в `Some`.
fn default_value(expr: &Expr, is_option: bool) -> TokenStream {
    let literal = match expr {
        Expr::Lit(ExprLit { lit, .. }) => Some(lit),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr: inner,
            ..
        }) => match inner.as_ref() {
            Expr::Lit(ExprLit { lit, .. }) => Some(lit),
            _ => None,
        },
        _ => None,
    };
    let value = match literal {
        Some(Lit::Int(_) | Lit::Float(_) | Lit::Bool(_)) => quote!(#expr),
        Some(_) => quote!(::core::convert::Into::into(#expr)),
        None => return quote!(::core::convert::Into::into(#expr)),
    };
    if is_option {
        quote!(::core::option::Option::Some(#value))
    } else {
        value
    }
}

fn raw_name(ident: &syn::Ident) -> String {
    let name = ident.to_string();
    name.strip_prefix("r#").map_or(name.clone(), str::to_owned)
}

fn field_validate(ident: &syn::Ident, ty: &Type, wire: &str, rules: &FieldRules) -> TokenStream {
    let mut checks = Vec::new();
    let p = quote!(::rustyapi::schema::__private);
    if let Some(min) = &rules.min_length {
        checks.push(quote!(#p::min_length(value, #min, loc, errors);));
    }
    if let Some(max) = &rules.max_length {
        checks.push(quote!(#p::max_length(value, #max, loc, errors);));
    }
    for (rule, bound) in [
        ("gt", &rules.gt),
        ("ge", &rules.ge),
        ("lt", &rules.lt),
        ("le", &rules.le),
    ] {
        if let Some(bound) = bound {
            let tokens = bound.tokens();
            let shown = &bound.shown;
            checks.push(quote!(#p::compare(value, #rule, #tokens, #shown, loc, errors);));
        }
    }
    if let Some(pattern) = &rules.pattern {
        checks.push(quote! {
            static PATTERN: #p::LazyLock<::rustyapi::regex::Regex> =
                #p::LazyLock::new(|| #p::regex(#pattern));
            #p::pattern(value, &PATTERN, loc, errors);
        });
    }
    if rules.email {
        checks.push(quote!(#p::email(value, loc, errors);));
    }
    if rules.url {
        checks.push(quote!(#p::url(value, loc, errors);));
    }
    // Ошибки типов (`gt` у строки) указывают на тип поля.
    let checks: Vec<TokenStream> = checks
        .into_iter()
        .map(|check| respan(check, ty.span()))
        .collect();
    let checks = if checks.is_empty() {
        quote!()
    } else if option_inner(ty).is_some() {
        quote! {
            if let ::core::option::Option::Some(value) = &self.#ident {
                #(#checks)*
            }
        }
    } else {
        quote! {
            {
                let value = &self.#ident;
                #(#checks)*
            }
        }
    };
    quote! {
        loc.push(::rustyapi::schema::LocItem::from(#wire));
        #checks
        (&&#p::Probe(&self.#ident)).__rustyapi_validate(loc, errors);
        loc.pop();
    }
}

fn field_schema(
    ty: &Type,
    ident: &syn::Ident,
    wire: &str,
    rules: &FieldRules,
    doc: &Option<String>,
    default_fn: Option<&syn::Ident>,
    has_serde_default: bool,
) -> TokenStream {
    let p = quote!(::rustyapi::schema::__private);
    let inner = option_inner(ty).unwrap_or(ty);
    let opt = |v: &Option<TokenStream>| match v {
        Some(t) => quote!(::core::option::Option::Some(#t)),
        None => quote!(::core::option::Option::None),
    };
    let min_length = opt(&rules.min_length.as_ref().map(|m| quote!(#m)));
    let max_length = opt(&rules.max_length.as_ref().map(|m| quote!(#m)));
    let bound = |b: &Option<NumBound>| {
        opt(&b.as_ref().map(|b| {
            let tokens = b.tokens();
            // Без `Numeric` у типа ошибку уже покажет проверка в `validate_into`.
            quote!((&&#p::SchemaProbe::<#inner>::new()).__rustyapi_bound(#tokens))
        }))
    };
    let (gt, ge, lt, le) = (
        bound(&rules.gt),
        bound(&rules.ge),
        bound(&rules.lt),
        bound(&rules.le),
    );
    let pattern = opt(&rules.pattern.as_ref().map(|s| quote!(#s)));
    let email = rules.email;
    let url = rules.url;
    let title = rules.title.clone().unwrap_or_else(|| title_case(wire));
    let description = rules.description.clone().or_else(|| doc.clone());
    let description = description
        .map(|d| quote!(map.insert("description".into(), ::rustyapi::serde_json::json!(#d));));
    let default =
        default_fn.map(|f| quote!(map.insert("default".into(), #p::to_json(&Self::#f()));));
    let example = rules
        .example
        .as_ref()
        .map(|e| quote!(map.insert("examples".into(), ::rustyapi::serde_json::json!([#e]));));
    let is_default = default_fn.is_some() || has_serde_default;
    let none_default = if option_inner(ty).is_some() && default_fn.is_none() {
        // Как `description: str | None = None` в Pydantic.
        quote! {
            if !map.contains_key("default") {
                map.insert("default".into(), ::rustyapi::serde_json::Value::Null);
            }
        }
    } else {
        quote!()
    };
    quote! {
        {
            let probe = &&#p::SchemaProbe::<#ty>::new();
            let mut field_schema = probe.__rustyapi_schema(components);
            #p::apply_constraints(
                &mut field_schema,
                &#p::Constraints {
                    min_length: #min_length,
                    max_length: #max_length,
                    gt: #gt,
                    ge: #ge,
                    lt: #lt,
                    le: #le,
                    pattern: #pattern,
                    email: #email,
                    url: #url,
                },
                components,
            );
            if let ::rustyapi::serde_json::Value::Object(map) = &mut field_schema {
                if !map.contains_key("$ref") {
                    map.insert("title".into(), ::rustyapi::serde_json::json!(#title));
                }
                #description
                #default
                #example
                #none_default
            }
            properties.insert(#wire.into(), field_schema);
            if !#is_default && probe.__rustyapi_required() {
                required.push(::rustyapi::serde_json::json!(#wire));
            }
            let _ = stringify!(#ident);
        }
    }
}

/// Переносит span на все токены (чтобы ошибка указывала на нужное место).
fn respan(tokens: TokenStream, span: Span) -> TokenStream {
    tokens
        .into_iter()
        .map(|mut tt| {
            if let proc_macro2::TokenTree::Group(group) = &tt {
                let mut new =
                    proc_macro2::Group::new(group.delimiter(), respan(group.stream(), span));
                new.set_span(span);
                tt = proc_macro2::TokenTree::Group(new);
            } else {
                tt.set_span(span);
            }
            tt
        })
        .collect()
}

/// `Option<T>` → `T` (синтаксически).
pub fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else { return None };
    if path.qself.is_some() {
        return None;
    }
    let last = path.path.segments.last()?;
    if last.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first()? {
        syn::GenericArgument::Type(inner) if args.args.len() == 1 => Some(inner),
        _ => None,
    }
}

pub fn title_case(name: &str) -> String {
    name.split(['_', ' ', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn doc_text(attrs: &[Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| match &a.meta {
            Meta::NameValue(nv) => match &nv.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => Some(s.value()),
                _ => None,
            },
            _ => None,
        })
        .map(|l| l.strip_prefix(' ').map(str::to_owned).unwrap_or(l))
        .collect();
    let text = lines.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn derive_names(attrs: &[Attribute]) -> Vec<String> {
    let mut names = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
        if let Ok(paths) =
            attr.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
        {
            for path in paths {
                if let Some(seg) = path.segments.last() {
                    names.push(seg.ident.to_string());
                }
            }
        }
    }
    names
}

fn missing_derives(attrs: &[Attribute], options: &Options) -> Vec<TokenStream> {
    let present = derive_names(attrs);
    let mut out = Vec::new();
    for (name, path) in [
        ("Serialize", quote!(::rustyapi::serde::Serialize)),
        ("Deserialize", quote!(::rustyapi::serde::Deserialize)),
        ("Debug", quote!(::core::fmt::Debug)),
        ("Clone", quote!(::core::clone::Clone)),
    ] {
        if !present.iter().any(|p| p == name) {
            out.push(path);
        }
    }
    out.extend(options.extra_derives.iter().cloned());
    out
}

fn serde_metas(attrs: &[Attribute]) -> syn::Result<Vec<Meta>> {
    let mut metas = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        let list = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        metas.extend(list);
    }
    Ok(metas)
}

fn has_serde_crate(attrs: &[Attribute]) -> bool {
    serde_metas(attrs)
        .unwrap_or_default()
        .iter()
        .any(|m| m.path().is_ident("crate"))
}

fn meta_str(meta: &Meta) -> Option<String> {
    match meta {
        Meta::NameValue(nv) => match &nv.value {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => Some(s.value()),
            _ => None,
        },
        // rename(deserialize = "...") — берём имя для десериализации.
        Meta::List(list) => {
            let inner = list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;
            inner
                .iter()
                .find(|m| m.path().is_ident("deserialize"))
                .and_then(meta_str)
        }
        Meta::Path(_) => None,
    }
}

#[derive(Default)]
struct ContainerSerde {
    rename_all: Option<String>,
    default: bool,
}

impl ContainerSerde {
    fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for meta in serde_metas(attrs)? {
            if meta.path().is_ident("rename_all") {
                let rule = meta_str(&meta).ok_or_else(|| {
                    syn::Error::new(meta.span(), "ожидается rename_all = \"...\"")
                })?;
                if !RENAME_RULES.contains(&rule.as_str()) {
                    return Err(syn::Error::new(
                        meta.span(),
                        format!(
                            "неизвестное правило rename_all / unknown rename_all rule: {rule:?}"
                        ),
                    ));
                }
                out.rename_all = Some(rule);
            } else if meta.path().is_ident("default") {
                out.default = true;
            }
        }
        Ok(out)
    }

    fn rename_field(&self, field: &str) -> String {
        match self.rename_all.as_deref() {
            None => field.to_owned(),
            Some(rule) => rename_field(rule, field),
        }
    }

    fn rename_variant(&self, variant: &str) -> String {
        match self.rename_all.as_deref() {
            None => variant.to_owned(),
            Some(rule) => rename_variant(rule, variant),
        }
    }
}

const RENAME_RULES: &[&str] = &[
    "lowercase",
    "UPPERCASE",
    "PascalCase",
    "camelCase",
    "snake_case",
    "SCREAMING_SNAKE_CASE",
    "kebab-case",
    "SCREAMING-KEBAB-CASE",
];

/// Как `serde_derive::RenameRule::apply_to_field` (поля — в snake_case).
fn rename_field(rule: &str, field: &str) -> String {
    match rule {
        "UPPERCASE" | "SCREAMING_SNAKE_CASE" => field.to_ascii_uppercase(),
        "PascalCase" => {
            let mut out = String::new();
            let mut capitalize = true;
            for c in field.chars() {
                if c == '_' {
                    capitalize = true;
                } else if capitalize {
                    out.push(c.to_ascii_uppercase());
                    capitalize = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
        "camelCase" => {
            let pascal = rename_field("PascalCase", field);
            let mut chars = pascal.chars();
            match chars.next() {
                Some(f) => f.to_ascii_lowercase().to_string() + chars.as_str(),
                None => pascal,
            }
        }
        "kebab-case" => field.replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => field.to_ascii_uppercase().replace('_', "-"),
        _ => field.to_owned(),
    }
}

/// Как `serde_derive::RenameRule::apply_to_variant` (варианты — в PascalCase).
fn rename_variant(rule: &str, variant: &str) -> String {
    match rule {
        "lowercase" => variant.to_ascii_lowercase(),
        "UPPERCASE" => variant.to_ascii_uppercase(),
        "camelCase" => {
            let mut chars = variant.chars();
            match chars.next() {
                Some(f) => f.to_ascii_lowercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
        "snake_case" => {
            let mut out = String::new();
            for (i, c) in variant.char_indices() {
                if i > 0 && c.is_uppercase() {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            }
            out
        }
        "SCREAMING_SNAKE_CASE" => rename_variant("snake_case", variant).to_ascii_uppercase(),
        "kebab-case" => rename_variant("snake_case", variant).replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => rename_variant("SCREAMING_SNAKE_CASE", variant).replace('_', "-"),
        _ => variant.to_owned(),
    }
}

#[derive(Default)]
struct FieldSerde {
    rename: Option<String>,
    skip: bool,
    skip_deserializing: bool,
    has_default: bool,
}

impl FieldSerde {
    fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for meta in serde_metas(attrs)? {
            let path = meta.path();
            if path.is_ident("rename") {
                out.rename = meta_str(&meta);
            } else if path.is_ident("skip") {
                out.skip = true;
            } else if path.is_ident("skip_deserializing") {
                out.skip_deserializing = true;
            } else if path.is_ident("default") {
                out.has_default = true;
            }
        }
        Ok(out)
    }
}

/// Числовая граница правила.
struct NumBound {
    value: NumValue,
    shown: String,
}

enum NumValue {
    Int(i128),
    Float(f64),
}

impl NumBound {
    fn tokens(&self) -> TokenStream {
        match self.value {
            NumValue::Int(i) => quote!(::rustyapi::schema::__private::Bound::Int(#i)),
            NumValue::Float(f) => quote!(::rustyapi::schema::__private::Bound::Float(#f)),
        }
    }
}

#[derive(Default)]
struct FieldRules {
    min_length: Option<usize>,
    max_length: Option<usize>,
    gt: Option<NumBound>,
    ge: Option<NumBound>,
    lt: Option<NumBound>,
    le: Option<NumBound>,
    pattern: Option<String>,
    email: bool,
    url: bool,
    default: Option<Expr>,
    example: Option<Expr>,
    title: Option<String>,
    description: Option<String>,
}

impl FieldRules {
    /// Разбирает и удаляет `#[field(...)]` из атрибутов поля.
    fn take_from(attrs: &mut Vec<Attribute>) -> syn::Result<Self> {
        let mut rules = Self::default();
        let mut seen: Vec<String> = Vec::new();
        let mut error: Option<syn::Error> = None;
        attrs.retain(|attr| {
            if !attr.path().is_ident("field") {
                return true;
            }
            if let Err(e) = rules.parse_attr(attr, &mut seen) {
                match &mut error {
                    Some(err) => err.combine(e),
                    None => error = Some(e),
                }
            }
            false
        });
        match error {
            Some(err) => Err(err),
            None => Ok(rules),
        }
    }

    fn parse_attr(&mut self, attr: &Attribute, seen: &mut Vec<String>) -> syn::Result<()> {
        let metas = match &attr.meta {
            Meta::List(list) => {
                Punctuated::<Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())?
            }
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "ожидается #[field(...)] / expected #[field(...)]\n\
                     подсказка / help: #[field(min_length = 1)]",
                ));
            }
        };
        for meta in metas {
            let key = meta
                .path()
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            if seen.contains(&key) {
                return Err(syn::Error::new(
                    meta.span(),
                    format!("правило `{key}` указано дважды / duplicate rule `{key}`"),
                ));
            }
            seen.push(key.clone());
            match key.as_str() {
                "email" | "url" => {
                    if !matches!(meta, Meta::Path(_)) {
                        return Err(syn::Error::new(
                            meta.span(),
                            format!(
                                "`{key}` — флаг без значения / `{key}` is a flag\nподсказка / help: #[field({key})]"
                            ),
                        ));
                    }
                    if key == "email" {
                        self.email = true;
                    } else {
                        self.url = true;
                    }
                }
                "min_length" | "max_length" => {
                    let value = name_value(&meta, &key)?;
                    let n = match value {
                        Expr::Lit(ExprLit {
                            lit: Lit::Int(i), ..
                        }) => i.base10_parse::<usize>()?,
                        other => {
                            return Err(syn::Error::new(
                                other.span(),
                                format!(
                                    "`{key}` — целое неотрицательное число / `{key}` must be a non-negative integer"
                                ),
                            ));
                        }
                    };
                    if key == "min_length" {
                        self.min_length = Some(n);
                    } else {
                        self.max_length = Some(n);
                    }
                }
                "gt" | "ge" | "lt" | "le" => {
                    let bound = parse_bound(name_value(&meta, &key)?, &key)?;
                    match key.as_str() {
                        "gt" => self.gt = Some(bound),
                        "ge" => self.ge = Some(bound),
                        "lt" => self.lt = Some(bound),
                        _ => self.le = Some(bound),
                    }
                }
                "pattern" => {
                    let value = name_value(&meta, &key)?;
                    let Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }) = value
                    else {
                        return Err(syn::Error::new(
                            value.span(),
                            "`pattern` — строка с регулярным выражением / `pattern` must be a string\n\
                             подсказка / help: pattern = r\"^[a-z]+$\"",
                        ));
                    };
                    if let Err(err) = regex::Regex::new(&s.value()) {
                        return Err(syn::Error::new(
                            s.span(),
                            format!("некорректное регулярное выражение / invalid regex: {err}"),
                        ));
                    }
                    self.pattern = Some(s.value());
                }
                "default" => self.default = Some(name_value(&meta, &key)?.clone()),
                "example" | "examples" => self.example = Some(name_value(&meta, &key)?.clone()),
                "title" | "description" => {
                    let value = name_value(&meta, &key)?;
                    let Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }) = value
                    else {
                        return Err(syn::Error::new(
                            value.span(),
                            format!("`{key}` должен быть строкой / `{key}` must be a string"),
                        ));
                    };
                    if key == "title" {
                        self.title = Some(s.value());
                    } else {
                        self.description = Some(s.value());
                    }
                }
                _ => {
                    let hint = match key.as_str() {
                        "min" | "minimum" => "\nподсказка / help: используйте `ge = ...`",
                        "max" | "maximum" => "\nподсказка / help: используйте `le = ...`",
                        "min_len" | "minlength" => {
                            "\nподсказка / help: используйте `min_length = ...`"
                        }
                        "max_len" | "maxlength" => {
                            "\nподсказка / help: используйте `max_length = ...`"
                        }
                        "regex" => "\nподсказка / help: используйте `pattern = \"...\"`",
                        _ => "",
                    };
                    return Err(syn::Error::new(
                        meta.path().span(),
                        format!(
                            "неизвестное правило `{key}` / unknown rule `{key}`; \
                             допустимые / expected one of: {FIELD_ALLOWED}{hint}"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn name_value<'a>(meta: &'a Meta, key: &str) -> syn::Result<&'a Expr> {
    match meta {
        Meta::NameValue(nv) => Ok(&nv.value),
        _ => Err(syn::Error::new(
            meta.span(),
            format!("ожидается `{key} = ...` / expected `{key} = ...`"),
        )),
    }
}

fn parse_bound(expr: &Expr, key: &str) -> syn::Result<NumBound> {
    let (negative, lit) = match expr {
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr,
            ..
        }) => (true, expr.as_ref()),
        other => (false, other),
    };
    let sign = if negative { "-" } else { "" };
    match lit {
        Expr::Lit(ExprLit {
            lit: Lit::Int(i), ..
        }) => {
            let v = i.base10_parse::<i128>()?;
            let v = if negative { -v } else { v };
            Ok(NumBound {
                value: NumValue::Int(v),
                shown: format!("{sign}{}", i.base10_digits()),
            })
        }
        Expr::Lit(ExprLit {
            lit: Lit::Float(f), ..
        }) => {
            let v = f.base10_parse::<f64>()?;
            let v = if negative { -v } else { v };
            Ok(NumBound {
                value: NumValue::Float(v),
                shown: format!("{sign}{}", f.base10_digits()),
            })
        }
        other => Err(syn::Error::new(
            other.span(),
            format!(
                "`{key}` — числовой литерал / `{key}` must be a number literal\nподсказка / help: {key} = 0"
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames() {
        assert_eq!(rename_field("camelCase", "owner_email"), "ownerEmail");
        assert_eq!(rename_field("PascalCase", "owner_email"), "OwnerEmail");
        assert_eq!(rename_field("kebab-case", "owner_email"), "owner-email");
        assert_eq!(rename_variant("snake_case", "InProgress"), "in_progress");
        assert_eq!(rename_variant("lowercase", "InProgress"), "inprogress");
        assert_eq!(
            rename_variant("SCREAMING-KEBAB-CASE", "InProgress"),
            "IN-PROGRESS"
        );
    }
}
