//! Загрузка файлов (`multipart/form-data`) — аналог `UploadFile` и `File()`/`Form()` из FastAPI.
//!
//! ```
//! use rustyapi::prelude::*;
//! use rustyapi::upload::{MultipartForm, UploadFile};
//!
//! /// Один файл (первый файл формы).
//! #[post("/upload")]
//! async fn upload(file: UploadFile) -> Json<Value> {
//!     Json(json!({ "filename": file.filename(), "size": file.size() }))
//! }
//!
//! /// Несколько полей.
//! #[post("/profile")]
//! async fn profile(form: MultipartForm) -> Result<Json<Value>> {
//!     let avatar = form.file("avatar")?;
//!     let name = form.require_text("name")?;
//!     Ok(Json(json!({ "name": name, "avatar": avatar.filename() })))
//! }
//! ```
//!
//! Размер тела ограничен `App::body_limit` (по умолчанию 2 МБ, как в axum).

use axum::body::Bytes;
use axum::extract::multipart::MultipartError;
use axum::extract::{FromRequest, Multipart, Request};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::error::HTTPException;
use crate::openapi::{OpenApiArg, Operation};
use crate::schema::{Components, ErrorDetail, LocItem, ValidationError};

/// Загруженный файл.
#[derive(Debug, Clone)]
pub struct UploadFile {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: Bytes,
}

impl UploadFile {
    /// Имя поля формы.
    pub fn field_name(&self) -> &str {
        &self.name
    }

    /// Имя файла от клиента. Не используйте его как путь на диске без проверки.
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// MIME-тип от клиента.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Размер в байтах.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Содержимое.
    pub fn bytes(&self) -> &Bytes {
        &self.data
    }

    /// Содержимое, забирая владение.
    pub fn into_bytes(self) -> Bytes {
        self.data
    }

    /// Содержимое как UTF-8 текст.
    pub fn text(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.data)
    }

    /// Сохраняет файл на диск.
    pub async fn save(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        tokio::fs::write(path, &self.data).await
    }
}

/// Все поля `multipart/form-data`: файлы и текстовые поля.
#[derive(Debug, Clone, Default)]
pub struct MultipartForm {
    files: Vec<UploadFile>,
    fields: Vec<(String, String)>,
}

fn missing(name: &str) -> ValidationError {
    ValidationError::single(ErrorDetail::new(
        "missing",
        vec![LocItem::from("body"), LocItem::from(name.to_owned())],
        "Field required",
        Value::Null,
    ))
}

impl MultipartForm {
    /// Первый файл с этим именем поля; нет — 422 `missing` (как в FastAPI).
    pub fn file(&self, name: &str) -> Result<&UploadFile, ValidationError> {
        self.files
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| missing(name))
    }

    /// Все файлы с этим именем поля (`files: list[UploadFile]`).
    pub fn files(&self, name: &str) -> Vec<&UploadFile> {
        self.files.iter().filter(|f| f.name == name).collect()
    }

    /// Все файлы формы.
    pub fn all_files(&self) -> &[UploadFile] {
        &self.files
    }

    /// Текстовое поле.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Обязательное текстовое поле; нет — 422 `missing`.
    pub fn require_text(&self, name: &str) -> Result<&str, ValidationError> {
        self.text(name).ok_or_else(|| missing(name))
    }

    #[allow(clippy::result_large_err)] // Response — ответ-ошибка как в экстракторах axum
    async fn read(request: Request) -> Result<Self, Response> {
        let is_multipart = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("multipart/form-data"));
        if !is_multipart {
            return Ok(Self::default());
        }
        let mut multipart = Multipart::from_request(request, &())
            .await
            .map_err(|_| bad_body())?;
        let mut form = Self::default();
        while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
            let name = field.name().unwrap_or_default().to_owned();
            let filename = field.file_name().map(str::to_owned);
            let content_type = field.content_type().map(str::to_owned);
            let data = field.bytes().await.map_err(multipart_error)?;
            match filename {
                Some(filename) => form.files.push(UploadFile {
                    name,
                    filename: Some(filename),
                    content_type,
                    data,
                }),
                None => form
                    .fields
                    .push((name, String::from_utf8_lossy(&data).into_owned())),
            }
        }
        Ok(form)
    }
}

fn bad_body() -> Response {
    HTTPException::new(400, "There was an error parsing the body").into_response()
}

fn multipart_error(error: MultipartError) -> Response {
    let status = error.status();
    if status == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        HTTPException::from_status(status).into_response()
    } else {
        bad_body()
    }
}

impl<S: Send + Sync> FromRequest<S> for MultipartForm {
    type Rejection = Response;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        Self::read(request).await
    }
}

impl<S: Send + Sync> FromRequest<S> for UploadFile {
    type Rejection = Response;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let form = MultipartForm::read(request).await?;
        form.files
            .into_iter()
            .next()
            .ok_or_else(|| missing("file").into_response())
    }
}

fn multipart_body(operation: &mut Operation, properties: Value, required: Value) {
    operation.request_body = Some(json!({
        "content": {"multipart/form-data": {"schema": {
            "type": "object",
            "properties": properties,
            "required": required,
        }}},
        "required": true,
    }));
}

impl OpenApiArg for UploadFile {
    fn doc_arg(operation: &mut Operation, _components: &mut Components) {
        multipart_body(
            operation,
            json!({"file": {"type": "string", "format": "binary", "title": "File"}}),
            json!(["file"]),
        );
    }
}

impl OpenApiArg for MultipartForm {
    fn doc_arg(operation: &mut Operation, _components: &mut Components) {
        operation.request_body = Some(json!({
            "content": {"multipart/form-data": {"schema": {"type": "object"}}},
            "required": true,
        }));
    }
}
