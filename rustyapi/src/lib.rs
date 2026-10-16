//! RustyAPI — веб-фреймворк на Rust в стиле FastAPI.
//!
//! «Пишется как FastAPI, работает как Rust». Пользователь подключает только `rustyapi`,
//! всё остальное реэкспортируется отсюда, а код из макросов ссылается только на `::rustyapi::...`.

pub mod app;
pub mod auth;
pub mod db;
pub mod error;
pub mod extract;
pub mod logging;
pub mod openapi;
pub mod routing;
pub mod schema;
pub mod status;

pub use app::App;
pub use error::{Error, HTTPException, Result};
pub use extract::{Form, Json, Path, Query};
pub use routing::{APIRouter, Endpoint};

// Процедурные макросы.
pub use rustyapi_macros;
pub use rustyapi_macros::{delete, get, head, main, options, patch, post, put, schema};
pub use schema::{ApiSchema, Validate, ValidationError};

/// Всё нужное для типичного приложения: `use rustyapi::prelude::*;`.
pub mod prelude {
    pub use crate::axum::http::StatusCode;
    pub use crate::serde_json::{Value, json};
    pub use crate::{
        APIRouter, ApiSchema, App, Form, HTTPException, Json, Path, Query, Result, Validate,
        ValidationError, delete, get, head, options, patch, post, put, schema, status,
    };
}

// Реэкспорт зависимостей: сгенерированный код и пользователь обращаются к ним через `::rustyapi::...`.
pub use argon2;
pub use axum;
pub use chrono;
pub use dotenvy;
pub use envy;
pub use jsonwebtoken;
pub use regex;
pub use serde;
pub use serde_json;
pub use serde_path_to_error;
pub use sqlx;
pub use tokio;
pub use tower_http;
pub use tracing;
pub use tracing_subscriber;
pub use uuid;

/// Не публичный API: используется кодом, который генерируют макросы.
#[doc(hidden)]
pub mod __private {
    /// Запускает async `main` на многопоточном рантайме tokio (для `#[rustyapi::main]`).
    pub fn block_on<O, F: std::future::Future<Output = O>>(future: F) -> O {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("не удалось запустить рантайм tokio / failed to build the tokio runtime")
            .block_on(future)
    }
}
