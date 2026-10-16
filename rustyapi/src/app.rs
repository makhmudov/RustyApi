//! Приложение `App` — аналог `FastAPI()`.

use std::any::Any;
use std::future::Future;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;

use crate::error::{Error, HTTPException, Result, internal_server_error};
use crate::logging::{self, ACCESS_TARGET};
use crate::routing::{APIRouter, Endpoint, PathMatcher, RouteInfo};

/// Хост по умолчанию (как у uvicorn).
pub const DEFAULT_HOST: &str = "127.0.0.1";
/// Порт по умолчанию (как у uvicorn).
pub const DEFAULT_PORT: u16 = 8000;

/// Приложение RustyAPI.
///
/// ```no_run
/// use rustyapi::prelude::*;
///
/// #[get("/")]
/// async fn root() -> Json<Value> {
///     Json(json!({ "message": "Hello" }))
/// }
///
/// #[rustyapi::main]
/// async fn main() -> rustyapi::Result<()> {
///     App::new().title("My API").route(root).run().await
/// }
/// ```
#[derive(Debug)]
pub struct App {
    info: AppInfo,
    router: APIRouter,
}

/// Метаданные приложения (пойдут в OpenAPI).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AppInfo {
    /// Название API.
    pub title: String,
    /// Версия API.
    pub version: String,
    /// Описание API.
    pub description: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Новое приложение: title `"RustyAPI"`, version `"0.1.0"` (как значения по умолчанию в FastAPI).
    pub fn new() -> Self {
        Self {
            info: AppInfo {
                title: "RustyAPI".to_owned(),
                version: "0.1.0".to_owned(),
                description: None,
            },
            router: APIRouter::new(),
        }
    }

    /// Название API.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.info.title = title.into();
        self
    }

    /// Версия API.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.info.version = version.into();
        self
    }

    /// Описание API.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.info.description = Some(description.into());
        self
    }

    /// Регистрирует эндпоинт.
    pub fn route(mut self, endpoint: impl Endpoint) -> Self {
        self.router = self.router.route(endpoint);
        self
    }

    /// Подключает [`APIRouter`].
    pub fn include_router(mut self, router: APIRouter) -> Self {
        self.router = self.router.include_router(router);
        self
    }

    /// Метаданные приложения.
    pub fn info(&self) -> &AppInfo {
        &self.info
    }

    /// Все маршруты приложения.
    pub fn routes(&self) -> Vec<RouteInfo> {
        self.router.routes()
    }

    /// Готовый `axum::Router` со всеми обработчиками ошибок и логами. Удобно для тестов
    /// (`tower::ServiceExt::oneshot`) и встраивания в существующее axum-приложение.
    pub fn into_router(self) -> axum::Router {
        let (router, matcher) = self.router.into_axum();
        router
            .fallback(move |request: Request| not_found(matcher.clone(), request))
            .layer(CatchPanicLayer::custom(handle_panic))
            .layer(axum::middleware::from_fn(access_log))
    }

    /// Запускает сервер на `HOST:PORT` из переменных окружения (по умолчанию `127.0.0.1:8000`).
    /// Останавливается корректно по Ctrl+C / SIGTERM.
    pub async fn run(self) -> Result<()> {
        logging::init();
        let host = std::env::var("HOST").unwrap_or_else(|_| DEFAULT_HOST.to_owned());
        let port = match std::env::var("PORT") {
            Ok(port) => port.parse::<u16>().map_err(|_| {
                Error::msg(format!(
                    "некорректный PORT={port:?}, ожидается число 0..65535 / invalid PORT"
                ))
            })?,
            Err(_) => DEFAULT_PORT,
        };
        let listener = match TcpListener::bind((host.as_str(), port)).await {
            Ok(listener) => listener,
            Err(err) => {
                tracing::error!(
                    "error while attempting to bind on address ('{host}', {port}): {err}"
                );
                return Err(err.into());
            }
        };
        self.serve(listener).await
    }

    /// Запускает сервер на готовом `TcpListener`. Останавливается по Ctrl+C / SIGTERM.
    pub async fn serve(self, listener: TcpListener) -> Result<()> {
        self.serve_with_shutdown(listener, shutdown_signal()).await
    }

    /// Запускает сервер и корректно останавливает его, когда завершится `signal`.
    pub async fn serve_with_shutdown<F>(self, listener: TcpListener, signal: F) -> Result<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        logging::init();
        let pid = std::process::id();
        let addr = listener.local_addr()?;
        tracing::info!("Started server process [{pid}]");
        tracing::info!("Waiting for application startup.");
        let router = self.into_router();
        tracing::info!("Application startup complete.");
        tracing::info!("RustyAPI running on http://{addr} (Press CTRL+C to quit)");

        let signal = async move {
            signal.await;
            tracing::info!("Shutting down");
        };
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(signal)
        .await?;

        tracing::info!("Waiting for application shutdown.");
        tracing::info!("Application shutdown complete.");
        tracing::info!("Finished server process [{pid}]");
        Ok(())
    }
}

/// 404 `{"detail": "Not Found"}`; если путь отличается от маршрута только завершающим `/` —
/// 307 Temporary Redirect, как `redirect_slashes` в FastAPI/Starlette.
async fn not_found(matcher: PathMatcher, request: Request) -> Response {
    if let Some(mut location) = matcher.redirect_target(request.uri().path()) {
        if let Some(query) = request.uri().query() {
            location.push('?');
            location.push_str(query);
        }
        if let Ok(location) = HeaderValue::try_from(location) {
            return (
                StatusCode::TEMPORARY_REDIRECT,
                [(header::LOCATION, location)],
            )
                .into_response();
        }
    }
    HTTPException::from_status(StatusCode::NOT_FOUND).into_response()
}

fn handle_panic(payload: Box<dyn Any + Send + 'static>) -> Response {
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned());
    tracing::error!("Exception in request handler: panicked: {message}");
    internal_server_error()
}

async fn access_log(request: Request, next: Next) -> impl IntoResponse {
    let client = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    let path = request
        .uri()
        .path_and_query()
        .map_or("/", |pq| pq.as_str())
        .to_owned();
    let request_line = format!("{} {path} {:?}", request.method(), request.version());
    let response = next.run(request).await;
    tracing::info!(
        target: ACCESS_TARGET,
        "{}",
        logging::access_line(client, &request_line, response.status())
    );
    response
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
