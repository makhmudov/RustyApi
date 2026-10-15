//! Продакшен-пример RustyAPI — сокращатель ссылок.
//!
//! * регистрация и вход по JWT (`/users`, `/token`), ссылки видит и удаляет только владелец;
//! * `GET /r/{code}` — редирект; адрес берётся из кэша (Redis), счётчик переходов растёт
//!   в фоновой задаче уже после ответа;
//! * настройки из окружения с валидацией, CORS, gzip, лимит тела, таймаут, rate limit;
//! * `/health` проверяет базу и кэш — для healthcheck в Docker и балансировщика;
//! * миграции при старте, корректное завершение по SIGTERM, Dockerfile и docker-compose.

pub mod links;
pub mod settings;
pub mod system;
pub mod users;

use std::time::Duration;

use rustyapi::App;
use rustyapi::cache::Cache;
use rustyapi::ratelimit::RateLimit;

pub use settings::Settings;

/// Маршруты и документация без инфраструктуры — удобно для тестов.
pub fn routes() -> App {
    App::new()
        .title("Shortener")
        .version(env!("CARGO_PKG_VERSION"))
        .description("Сокращатель ссылок на RustyAPI — пример продакшен-проекта.")
        .include_router(system::router())
        .include_router(users::router())
        .include_router(links::router())
}

/// Полностью собранное приложение: база, кэш и все слои по настройкам.
pub async fn app(settings: &Settings) -> rustyapi::Result<App> {
    rustyapi::auth::set_secret_key(settings.secret_key.clone());
    let cache = match &settings.redis_url {
        Some(url) => Cache::redis(url).await?.with_prefix("shortener:"),
        None => Cache::memory(),
    };
    let mut rate_limit = RateLimit::per_minute(settings.rate_limit_per_minute);
    if settings.trust_proxy {
        rate_limit = rate_limit.trust_forwarded_for();
    }
    Ok(routes()
        .database_from_env()
        .migrations(settings.migrations_dir.clone())
        .cache(cache)
        .cors_origins(settings.cors_origins.clone())
        .gzip()
        .body_limit(settings.body_limit_bytes)
        .timeout(Duration::from_secs(settings.request_timeout_secs))
        .rate_limit(rate_limit))
}
