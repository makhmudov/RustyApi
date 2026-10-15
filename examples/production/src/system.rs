//! Служебные эндпоинты.

use std::time::Duration;

use rustyapi::prelude::*;
use rustyapi::tokio::time::timeout;

/// Сколько ждать базу и кэш: зависший сервис должен быстро получить 503, а не висеть.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Информация о сервисе.
#[get("/", tags = ["system"])]
async fn root() -> Json<Value> {
    Json(json!({ "service": "shortener", "version": env!("CARGO_PKG_VERSION"), "docs": "/docs" }))
}

/// Проверка готовности: база и кэш отвечают. 503, если что-то недоступно.
#[get("/health", tags = ["system"])]
async fn health(db: Db, cache: Cache) -> (StatusCode, Json<Value>) {
    let database = rustyapi::sqlx::query("SELECT 1").execute(&*db);
    let database = matches!(timeout(CHECK_TIMEOUT, database).await, Ok(Ok(_)));
    let cache = cache.get::<String>("health");
    let cache_ok = matches!(timeout(CHECK_TIMEOUT, cache).await, Ok(Ok(_)));
    let status = if database && cache_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let word = |ok: bool| if ok { "ok" } else { "unavailable" };
    let body = json!({
        "status": word(database && cache_ok),
        "database": word(database),
        "cache": word(cache_ok),
    });
    (status, Json(body))
}

pub fn router() -> APIRouter {
    APIRouter::new().route(root).route(health)
}
