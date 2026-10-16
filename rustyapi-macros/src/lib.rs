//! Процедурные макросы RustyAPI: `#[get]`/`#[post]`/..., `#[schema]`, `#[model]`, `#[rustyapi::main]`.
//!
//! Сгенерированный код ссылается только на `::rustyapi::...`.

use proc_macro::TokenStream;

mod main_attr;
mod model;
mod route;
mod schema;

macro_rules! route_macros {
    ($($name:ident => $method:ident;)*) => {
        $(
            #[doc = concat!(
                "Эндпоинт `", stringify!($method), "`: `#[", stringify!($name),
                "(\"/path\", status_code = 201, tags = [\"items\"], summary = \"...\", deprecated)]`."
            )]
            #[proc_macro_attribute]
            pub fn $name(args: TokenStream, item: TokenStream) -> TokenStream {
                route::expand(stringify!($name), stringify!($method), args.into(), item.into())
                    .unwrap_or_else(syn::Error::into_compile_error)
                    .into()
            }
        )*
    };
}

route_macros! {
    get => GET;
    post => POST;
    put => PUT;
    patch => PATCH;
    delete => DELETE;
    head => HEAD;
    options => OPTIONS;
}

/// WebSocket-эндпоинт: `#[websocket("/ws")] async fn chat(mut socket: WebSocket, ...)`.
///
/// Первый аргумент — `WebSocket`, остальные — экстракторы (`Path`, `Query`, `Auth`, `Depends`...),
/// они выполняются до установки соединения. В OpenAPI не попадает.
#[proc_macro_attribute]
pub fn websocket(args: TokenStream, item: TokenStream) -> TokenStream {
    route::expand_websocket(args.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Точка входа приложения: запускает async `main` на рантайме tokio (через `::rustyapi::tokio`).
///
/// ```ignore
/// #[rustyapi::main]
/// async fn main() -> rustyapi::Result<()> {
///     App::new().route(root).run().await
/// }
/// ```
#[proc_macro_attribute]
pub fn main(args: TokenStream, item: TokenStream) -> TokenStream {
    let item: proc_macro2::TokenStream = item.into();
    main_attr::expand(args.into(), item.clone())
        .unwrap_or_else(|err| main_attr::with_stub(err, item))
        .into()
}

/// Схема данных — аналог `pydantic.BaseModel`.
///
/// Добавляет `Serialize`/`Deserialize` (через `::rustyapi::serde`), `Debug` и `Clone`
/// (если их нет в `#[derive]`), реализует `Validate` и `ApiSchema`. Правила полей:
/// `#[field(min_length = 1, max_length = 100, gt = 0, ge, lt, le, pattern = "...", email, url,
/// default = ..., example = ...)]`.
///
/// ```ignore
/// #[schema]
/// struct ItemCreate {
///     #[field(min_length = 1, max_length = 100)]
///     name: String,
///     #[field(gt = 0)]
///     price: f64,
///     description: Option<String>,
/// }
/// ```
#[proc_macro_attribute]
pub fn schema(args: TokenStream, item: TokenStream) -> TokenStream {
    schema::expand(args.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Таблица PostgreSQL: `#[schema]` + `FromRow` + трейт `Model` (CRUD и построители запросов).
///
/// ```ignore
/// #[model(table = "todos")]
/// struct Todo {
///     #[primary_key]
///     id: i64,
///     title: String,
///     done: bool,
/// }
/// ```
#[proc_macro_attribute]
pub fn model(args: TokenStream, item: TokenStream) -> TokenStream {
    model::expand(args.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
