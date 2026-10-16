//! Фоновые задачи — аналог `BackgroundTasks` из FastAPI: выполняются после ответа.
//!
//! ```
//! use rustyapi::prelude::*;
//!
//! #[post("/send")]
//! async fn send(tasks: BackgroundTasks) -> Json<Value> {
//!     tasks.add(async {
//!         // письмо, вебхук, пересчёт кэша...
//!     });
//!     Json(json!({ "queued": true }))
//! }
//! ```

use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::extract::{FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

use crate::error::HTTPException;

type Task = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Очередь задач запроса. Задачи запускаются по порядку после того, как обработчик вернул
/// успешный ответ (статус < 400); при ошибке, как в FastAPI, не запускаются.
#[derive(Clone, Default)]
pub struct BackgroundTasks {
    tasks: Arc<Mutex<Vec<Task>>>,
}

impl std::fmt::Debug for BackgroundTasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundTasks")
            .field("len", &self.len())
            .finish()
    }
}

/// Результат фоновой задачи: `()` или `Result` (ошибка пишется в лог).
pub trait TaskOutput: Send + 'static {
    /// Логирует ошибку, если она есть.
    fn report(self);
}

impl TaskOutput for () {
    fn report(self) {}
}

impl<T: Send + 'static, E: Display + Send + 'static> TaskOutput for Result<T, E> {
    fn report(self) {
        if let Err(error) = self {
            tracing::error!("Background task failed: {error}");
        }
    }
}

impl BackgroundTasks {
    /// Добавляет задачу. Результат `Err` попадёт в лог.
    pub fn add<F>(&self, task: F)
    where
        F: Future + Send + 'static,
        F::Output: TaskOutput,
    {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.push(Box::pin(async move { task.await.report() }));
        }
    }

    /// Сколько задач в очереди.
    pub fn len(&self) -> usize {
        self.tasks.lock().map_or(0, |t| t.len())
    }

    /// Пуста ли очередь.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn take(&self) -> Vec<Task> {
        self.tasks
            .lock()
            .map(|mut t| std::mem::take(&mut *t))
            .unwrap_or_default()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for BackgroundTasks {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<BackgroundTasks>() {
            Some(tasks) => Ok(tasks.clone()),
            None => {
                tracing::error!(
                    "BackgroundTasks: нет слоя App (используйте App::into_router) / \
                     BackgroundTasks requires App::into_router"
                );
                Err(HTTPException::from_status(500).into_response())
            }
        }
    }
}

/// Начало запроса: кладёт пустую очередь в extensions (вызывается слоем `App`).
pub(crate) fn begin(request: &mut Request) -> BackgroundTasks {
    let tasks = BackgroundTasks::default();
    request.extensions_mut().insert(tasks.clone());
    tasks
}

/// Конец запроса: запускает задачи, если ответ успешный (статус < 400), как в FastAPI.
pub(crate) fn finish(tasks: BackgroundTasks, status: StatusCode) {
    let pending = tasks.take();
    if pending.is_empty() || status.is_client_error() || status.is_server_error() {
        return;
    }
    tokio::spawn(async move {
        for task in pending {
            // Паника одной задачи не отменяет следующие.
            if let Err(error) = tokio::spawn(task).await {
                tracing::error!("Background task panicked: {error}");
            }
        }
    });
}
