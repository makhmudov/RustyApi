//! Зависимости — аналог `Depends` из FastAPI.
//!
//! ```
//! use rustyapi::prelude::*;
//! use rustyapi::depends::{Dependency, RequestCtx};
//!
//! /// Параметры пагинации, общие для многих эндпоинтов.
//! #[derive(Clone)]
//! struct Pagination {
//!     limit: i64,
//! }
//!
//! impl Dependency for Pagination {
//!     async fn resolve(ctx: &mut RequestCtx<'_>) -> rustyapi::Result<Self> {
//!         let limit = ctx
//!             .header("x-limit")
//!             .and_then(|v| v.parse().ok())
//!             .unwrap_or(20);
//!         Ok(Pagination { limit })
//!     }
//! }
//!
//! #[get("/items")]
//! async fn list_items(Depends(page): Depends<Pagination>) -> Json<Value> {
//!     Json(json!({ "limit": page.limit }))
//! }
//! ```

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

use crate::error::{Error, Result};
use crate::openapi::{OpenApiArg, Operation};
use crate::schema::Components;

/// Зависимость: значение, которое вычисляется из запроса перед вызовом обработчика.
///
/// Результат кэшируется на время запроса (как `use_cache=True` в FastAPI): если `CurrentUser`
/// нужен двум зависимостям одного эндпоинта, он загрузится один раз.
pub trait Dependency: Clone + Send + Sync + 'static {
    /// Вычисляет значение. Ошибка (`HTTPException`, `DbError`, ...) сразу становится ответом.
    fn resolve(ctx: &mut RequestCtx<'_>) -> impl Future<Output = Result<Self>> + Send;

    /// Описание в OpenAPI (например, требование авторизации). По умолчанию ничего.
    fn openapi(operation: &mut Operation, components: &mut Components) {
        let _ = (operation, components);
    }
}

/// Контекст запроса для [`Dependency::resolve`]: доступ к заголовкам, URI и любым экстракторам.
pub struct RequestCtx<'a> {
    parts: &'a mut Parts,
}

impl<'a> RequestCtx<'a> {
    /// Контекст поверх частей запроса.
    pub fn new(parts: &'a mut Parts) -> Self {
        Self { parts }
    }

    /// Вызывает экстрактор: `ctx.extract::<Db>().await?`, `ctx.extract::<Auth>().await?`,
    /// `ctx.extract::<Depends<CurrentUser>>().await?`.
    pub async fn extract<E>(&mut self) -> Result<E>
    where
        E: FromRequestParts<()>,
    {
        E::from_request_parts(self.parts, &())
            .await
            .map_err(|rejection| Error::from(RejectionError::new(rejection.into_response())))
    }

    /// Значение заголовка (если это валидная строка).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.parts.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Части запроса (метод, URI, заголовки, extensions).
    pub fn parts(&mut self) -> &mut Parts {
        self.parts
    }
}

/// Ответ-отказ экстрактора внутри [`Error`] (отдаётся клиенту как есть).
pub(crate) struct RejectionError(Mutex<Option<Response>>);

impl RejectionError {
    pub(crate) fn new(response: Response) -> Self {
        Self(Mutex::new(Some(response)))
    }

    pub(crate) fn take(&self) -> Option<Response> {
        self.0.lock().ok().and_then(|mut r| r.take())
    }
}

impl std::fmt::Debug for RejectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RejectionError")
    }
}

impl std::fmt::Display for RejectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("request rejected by an extractor")
    }
}

impl std::error::Error for RejectionError {}

/// Значение зависимости в аргументе обработчика: `Depends(user): Depends<CurrentUser>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Depends<T>(pub T);

impl<T> Depends<T> {
    /// Внутреннее значение.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Depends<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Depends<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Кэш зависимости в extensions запроса.
#[derive(Clone)]
struct Cached<T>(T);

impl<T: Dependency, S: Send + Sync> FromRequestParts<S> for Depends<T> {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Response> {
        if let Some(Cached(value)) = parts.extensions.get::<Cached<T>>() {
            return Ok(Depends(value.clone()));
        }
        let value = T::resolve(&mut RequestCtx::new(parts))
            .await
            .map_err(IntoResponse::into_response)?;
        parts.extensions.insert(Cached(value.clone()));
        Ok(Depends(value))
    }
}

impl<T: Dependency> OpenApiArg for Depends<T> {
    fn doc_arg(operation: &mut Operation, components: &mut Components) {
        T::openapi(operation, components);
    }
}
