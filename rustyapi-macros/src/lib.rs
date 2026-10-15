//! Процедурные макросы RustyAPI: `#[get]`/`#[post]`/..., `#[schema]`, `#[model]`, `#[rustyapi::main]`.
//!
//! Сгенерированный код ссылается только на `::rustyapi::...`.

use proc_macro::TokenStream;

mod main_attr;
mod route;

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
