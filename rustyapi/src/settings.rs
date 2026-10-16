//! Настройки из переменных окружения и `.env` — аналог `pydantic-settings`.
//!
//! ```no_run
//! use rustyapi::prelude::*;
//!
//! #[schema]
//! struct Settings {
//!     database_url: String,            // DATABASE_URL, обязательна
//!     #[field(default = 8000, ge = 1)]
//!     port: u16,                       // PORT, по умолчанию 8000
//!     #[field(default = Vec::new())]
//!     cors_origins: Vec<String>,       // CORS_ORIGINS=http://a.com,http://b.com
//!     debug: Option<bool>,
//! }
//!
//! # fn main() -> rustyapi::Result<()> {
//! let settings: Settings = rustyapi::settings()?;
//! # Ok(()) }
//! ```

use serde::de::DeserializeOwned;

use crate::error::{Error, Result};
use crate::schema::Validate;

/// Читает `.env` (если есть) и окружение в структуру. Имена переменных — имена полей в верхнем
/// регистре (`database_url` ← `DATABASE_URL`); списки — через запятую. После разбора выполняются
/// правила `#[field(...)]`.
pub fn settings<T: DeserializeOwned + Validate>() -> Result<T> {
    let _ = dotenvy::dotenv();
    settings_from(std::env::vars())
}

/// То же из заданного набора переменных (для тестов).
pub fn settings_from<T, I>(vars: I) -> Result<T>
where
    T: DeserializeOwned + Validate,
    I: IntoIterator<Item = (String, String)>,
{
    let value: T = envy::from_iter(vars).map_err(|error| {
        Error::msg(match error {
            envy::Error::MissingValue(field) => format!(
                "настройки: не задана переменная окружения {} / settings: missing environment variable",
                field.to_uppercase()
            ),
            envy::Error::Custom(message) => format!("настройки / settings: {message}"),
        })
    })?;
    value.validate().map_err(|errors| {
        let details: Vec<String> = errors
            .errors()
            .iter()
            .map(|e| {
                let name = e
                    .loc
                    .iter()
                    .map(|l| l.to_string().to_uppercase())
                    .collect::<Vec<_>>()
                    .join(".");
                format!("{name}: {}", e.msg)
            })
            .collect();
        Error::msg(format!("настройки / settings: {}", details.join("; ")))
    })?;
    Ok(value)
}
