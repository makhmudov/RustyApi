//! `HTTPException`, общий тип ошибки [`Error`] и ответы об ошибках в формате FastAPI.

use std::fmt;

use axum::http::header::{HeaderName, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::status::IntoStatusCode;

/// `Result` по умолчанию с ошибкой [`Error`]: подходит и для обработчиков, и для `main`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// HTTP-ошибка как в FastAPI: ответ `{"detail": ...}` с заданным статусом и заголовками.
///
/// ```
/// use rustyapi::HTTPException;
///
/// let exc = HTTPException::new(404, "Item not found");
/// let auth = HTTPException::new(401, "Not authenticated").with_header("WWW-Authenticate", "Bearer");
/// # let _ = (exc, auth);
/// ```
#[derive(Debug, Clone)]
pub struct HTTPException {
    // В коробке: `Result<T, HTTPException>` остаётся маленьким (clippy::result_large_err).
    inner: Box<ExceptionInner>,
}

#[derive(Debug, Clone)]
struct ExceptionInner {
    status_code: StatusCode,
    detail: Value,
    headers: HeaderMap,
}

impl HTTPException {
    /// Ошибка со статусом и `detail` (строка, число, `json!({...})` — что угодно, превращаемое в JSON).
    pub fn new(status_code: impl IntoStatusCode, detail: impl Into<Value>) -> Self {
        Self {
            inner: Box::new(ExceptionInner {
                status_code: status_code.into_status_code(),
                detail: detail.into(),
                headers: HeaderMap::new(),
            }),
        }
    }

    /// Ошибка, у которой `detail` — стандартная фраза статуса (`404` → `"Not Found"`), как в FastAPI.
    pub fn from_status(status_code: impl IntoStatusCode) -> Self {
        let status_code = status_code.into_status_code();
        let detail = status_code.canonical_reason().unwrap_or("Unknown Status");
        Self::new(status_code, detail)
    }

    /// Добавляет заголовок к ответу (аналог `headers={...}` в FastAPI).
    ///
    /// # Panics
    /// Если имя или значение заголовка некорректны.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        let name = HeaderName::try_from(name).unwrap_or_else(|_| {
            panic!("некорректное имя заголовка / invalid header name: {name:?}")
        });
        let value = HeaderValue::try_from(value).unwrap_or_else(|_| {
            panic!("некорректное значение заголовка / invalid header value: {value:?}")
        });
        self.inner.headers.append(name, value);
        self
    }

    /// Статус-код ответа.
    pub fn status_code(&self) -> StatusCode {
        self.inner.status_code
    }

    /// Поле `detail` ответа.
    pub fn detail(&self) -> &Value {
        &self.inner.detail
    }

    /// Дополнительные заголовки ответа.
    pub fn headers(&self) -> &HeaderMap {
        &self.inner.headers
    }
}

impl fmt::Display for HTTPException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.inner.status_code.as_u16();
        match &self.inner.detail {
            Value::String(s) => write!(f, "{code}: {s}"),
            other => write!(f, "{code}: {other}"),
        }
    }
}

impl std::error::Error for HTTPException {}

impl IntoResponse for HTTPException {
    fn into_response(self) -> Response {
        let ExceptionInner {
            status_code,
            detail,
            headers,
        } = *self.inner;
        (
            status_code,
            headers,
            axum::Json(json!({ "detail": detail })),
        )
            .into_response()
    }
}

/// Общая ошибка RustyAPI (по духу как `anyhow::Error`).
///
/// В неё через `?` превращается любая `std::error::Error`. В ответе:
/// [`HTTPException`] отдаётся как есть, всё остальное — `500 {"detail": "Internal Server Error"}`
/// с записью в лог.
pub struct Error {
    inner: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl Error {
    /// Ошибка из произвольного сообщения.
    pub fn msg(message: impl fmt::Display) -> Self {
        Self {
            inner: Box::new(MessageError(message.to_string())),
        }
    }

    /// Если внутри ошибка типа `E` — ссылка на неё.
    pub fn downcast_ref<E: std::error::Error + 'static>(&self) -> Option<&E> {
        self.inner.downcast_ref()
    }

    /// Если внутри [`HTTPException`] — ссылка на неё.
    pub fn as_http_exception(&self) -> Option<&HTTPException> {
        self.inner.downcast_ref()
    }

    /// Исходная ошибка.
    pub fn inner(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        &*self.inner
    }
}

impl<E> From<E> for Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(error: E) -> Self {
        Self {
            inner: Box::new(error),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl fmt::Debug for Error {
    // `main` печатает ошибку через Debug — показываем понятное сообщение и цепочку причин.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)?;
        let mut source = self.inner.source();
        while let Some(cause) = source {
            write!(f, "\n  причина / caused by: {cause}")?;
            source = cause.source();
        }
        Ok(())
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let other = match self.inner.downcast::<HTTPException>() {
            Ok(exc) => return exc.into_response(),
            Err(other) => other,
        };
        match other.downcast::<crate::schema::ValidationError>() {
            Ok(err) => err.into_response(),
            Err(other) => {
                tracing::error!("Exception in request handler: {other}");
                internal_server_error()
            }
        }
    }
}

#[derive(Debug)]
struct MessageError(String);

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MessageError {}

/// `500 {"detail": "Internal Server Error"}`.
pub(crate) fn internal_server_error() -> Response {
    HTTPException::from_status(StatusCode::INTERNAL_SERVER_ERROR).into_response()
}
