//! Кэш: в памяти процесса или в Redis (фича `redis`). Значения хранятся как JSON.
//!
//! ```
//! use rustyapi::prelude::*;
//! use std::time::Duration;
//!
//! #[get("/stats")]
//! async fn stats(cache: Cache) -> Result<Json<Value>> {
//!     let value = cache
//!         .get_or_set("stats", Duration::from_secs(60), || async {
//!             Ok::<_, rustyapi::Error>(json!({ "users": 42 })) // тяжёлый запрос
//!         })
//!         .await?;
//!     Ok(Json(value))
//! }
//!
//! let app = App::new().cache(Cache::memory()).route(stats);
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, HTTPException, Result};

/// Кэш приложения. Клонируется дёшево; в обработчике — экстрактор `cache: Cache`.
#[derive(Clone)]
pub struct Cache {
    backend: Backend,
    prefix: String,
}

#[derive(Clone)]
enum Backend {
    Memory(Arc<Mutex<Memory>>),
    #[cfg(feature = "redis")]
    Redis(redis::aio::ConnectionManager),
}

#[derive(Default)]
struct Memory {
    entries: HashMap<String, (String, Option<Instant>)>,
    writes: usize,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match &self.backend {
            Backend::Memory(_) => "memory",
            #[cfg(feature = "redis")]
            Backend::Redis(_) => "redis",
        };
        f.debug_struct("Cache")
            .field("backend", &kind)
            .field("prefix", &self.prefix)
            .finish()
    }
}

impl Cache {
    /// Кэш в памяти процесса (для одного экземпляра приложения и тестов).
    pub fn memory() -> Self {
        Self {
            backend: Backend::Memory(Arc::default()),
            prefix: String::new(),
        }
    }

    /// Кэш в Redis: `Cache::redis("redis://127.0.0.1/").await?`.
    #[cfg(feature = "redis")]
    pub async fn redis(url: &str) -> Result<Self> {
        let client =
            redis::Client::open(url).map_err(|e| Error::msg(format!("Redis {url:?}: {e}")))?;
        let manager = redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|e| Error::msg(format!("Redis {url:?}: {e}")))?;
        Ok(Self {
            backend: Backend::Redis(manager),
            prefix: String::new(),
        })
    }

    /// Префикс всех ключей (например, имя приложения в общем Redis).
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    fn key(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }

    /// Значение по ключу; `None`, если его нет, оно истекло или не подходит к типу `T`.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw = self.get_raw(&self.key(key)).await?;
        Ok(raw.and_then(|raw| serde_json::from_str(&raw).ok()))
    }

    /// Записывает значение; `ttl = None` — без срока.
    pub async fn set<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl: impl Into<Option<Duration>>,
    ) -> Result<()> {
        let raw = serde_json::to_string(value)?;
        self.set_raw(self.key(key), raw, ttl.into()).await
    }

    /// Удаляет ключ.
    pub async fn delete(&self, key: &str) -> Result<()> {
        let key = self.key(key);
        match &self.backend {
            Backend::Memory(memory) => {
                lock(memory).entries.remove(&key);
                Ok(())
            }
            #[cfg(feature = "redis")]
            Backend::Redis(manager) => {
                let mut conn = manager.clone();
                redis::cmd("DEL")
                    .arg(&key)
                    .query_async::<()>(&mut conn)
                    .await
                    .map_err(redis_error)
            }
        }
    }

    /// Значение из кэша или результат `compute` (он же записывается в кэш на `ttl`).
    pub async fn get_or_set<T, F, Fut, E>(&self, key: &str, ttl: Duration, compute: F) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = std::result::Result<T, E>>,
        E: Into<Error>,
    {
        if let Some(value) = self.get(key).await? {
            return Ok(value);
        }
        let value = compute().await.map_err(Into::into)?;
        self.set(key, &value, ttl).await?;
        Ok(value)
    }

    async fn get_raw(&self, key: &str) -> Result<Option<String>> {
        match &self.backend {
            Backend::Memory(memory) => {
                let mut memory = lock(memory);
                let expired = memory
                    .entries
                    .get(key)
                    .is_some_and(|(_, expires)| expires.is_some_and(|at| at <= Instant::now()));
                if expired {
                    memory.entries.remove(key);
                }
                Ok(memory.entries.get(key).map(|(value, _)| value.clone()))
            }
            #[cfg(feature = "redis")]
            Backend::Redis(manager) => {
                let mut conn = manager.clone();
                redis::cmd("GET")
                    .arg(key)
                    .query_async::<Option<String>>(&mut conn)
                    .await
                    .map_err(redis_error)
            }
        }
    }

    async fn set_raw(&self, key: String, value: String, ttl: Option<Duration>) -> Result<()> {
        match &self.backend {
            Backend::Memory(memory) => {
                let mut memory = lock(memory);
                memory
                    .entries
                    .insert(key, (value, ttl.map(|ttl| Instant::now() + ttl)));
                memory.writes += 1;
                // Изредка чистим истёкшие записи, чтобы память не росла без ограничений.
                if memory.writes % 1024 == 0 {
                    let now = Instant::now();
                    memory
                        .entries
                        .retain(|_, (_, expires)| expires.is_none_or(|at| at > now));
                }
                Ok(())
            }
            #[cfg(feature = "redis")]
            Backend::Redis(manager) => {
                let mut conn = manager.clone();
                let mut cmd = redis::cmd("SET");
                cmd.arg(&key).arg(value);
                if let Some(ttl) = ttl {
                    cmd.arg("PX").arg(ttl.as_millis().max(1) as u64);
                }
                cmd.query_async::<()>(&mut conn).await.map_err(redis_error)
            }
        }
    }
}

fn lock(memory: &Mutex<Memory>) -> std::sync::MutexGuard<'_, Memory> {
    memory
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "redis")]
fn redis_error(error: redis::RedisError) -> Error {
    Error::msg(format!("Redis: {error}"))
}

impl<S: Send + Sync> FromRequestParts<S> for Cache {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<Cache>() {
            Some(cache) => Ok(cache.clone()),
            None => {
                tracing::error!(
                    "кэш не подключён: добавьте App::cache(Cache::memory()) / no cache configured"
                );
                Err(HTTPException::from_status(500).into_response())
            }
        }
    }
}
