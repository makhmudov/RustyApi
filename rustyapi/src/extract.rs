//! Экстракторы `Json`, `Path`, `Query`, `Form`.
//!
//! Пока это экстракторы axum как есть. На этапе 2 они станут своими newtype с ошибками 422
//! в формате FastAPI; пользовательский код при этом не изменится.

pub use axum::Form;
pub use axum::Json;
pub use axum::extract::{Path, Query};
