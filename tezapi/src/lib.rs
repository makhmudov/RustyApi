//! Tez — веб-фреймворк на Rust в стиле FastAPI.
//!
//! «Пишется как FastAPI, работает как Rust». Пользователь подключает только `tezapi`,
//! всё остальное реэкспортируется отсюда, а код из макросов ссылается только на `::tezapi::...`.

pub mod app;
pub mod auth;
pub mod db;
pub mod error;
pub mod extract;
pub mod openapi;
pub mod routing;
pub mod schema;

// Реэкспорт процедурных макросов.
pub use tezapi_macros;

// Реэкспорт зависимостей: сгенерированный код и пользователь обращаются к ним через `::tezapi::...`.
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
