//! Настройки из окружения и `.env` — аналог `BaseSettings` из pydantic-settings.

use rustyapi::prelude::*;

/// Все параметры сервиса. Ошибка в любом из них — понятное сообщение при старте, а не 500 в рантайме.
#[schema]
pub struct Settings {
    /// Ключ подписи JWT. Не короче 32 символов: `openssl rand -hex 32`.
    #[field(min_length = 32)]
    pub secret_key: String,
    /// Redis для кэша редиректов. Без него — кэш в памяти процесса.
    pub redis_url: Option<String>,
    /// Источники фронтенда через запятую.
    #[field(default = Vec::new())]
    pub cors_origins: Vec<String>,
    /// Запросов в минуту с одного IP.
    #[field(default = 120, ge = 1)]
    pub rate_limit_per_minute: u32,
    /// Таймаут обработки запроса (дальше — 504).
    #[field(default = 10, ge = 1, le = 300)]
    pub request_timeout_secs: u64,
    /// Максимальный размер тела запроса (дальше — 413).
    #[field(default = 65536, ge = 1024)]
    pub body_limit_bytes: usize,
    /// Брать IP клиента из `X-Forwarded-For` (только за своим reverse proxy!).
    #[field(default = false)]
    pub trust_proxy: bool,
    /// Каталог миграций. В Docker-образе — `/app/migrations`.
    #[field(default = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations").to_owned())]
    pub migrations_dir: String,
}
