//! Приложение `App` — аналог `FastAPI()`.

use std::any::Any;
use std::future::Future;
use std::net::SocketAddr;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use serde_json::Value;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;

use crate::db::{self, Db};
use crate::error::{Error, HTTPException, Result, internal_server_error};
use crate::logging::{self, ACCESS_TARGET};
use crate::openapi::SecurityScheme;
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
    docs: DocsConfig,
    security_schemes: Vec<SecurityScheme>,
    database: Option<Result<Db, String>>,
    migrations: Option<String>,
    token_url: String,
}

/// Адреса документации (как `openapi_url`, `docs_url`, `redoc_url` в FastAPI).
#[derive(Debug, Clone)]
struct DocsConfig {
    openapi_url: Option<String>,
    docs_url: Option<String>,
    redoc_url: Option<String>,
    oauth2_redirect_url: Option<String>,
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
            docs: DocsConfig {
                openapi_url: Some("/openapi.json".to_owned()),
                docs_url: Some("/docs".to_owned()),
                redoc_url: Some("/redoc".to_owned()),
                oauth2_redirect_url: Some("/docs/oauth2-redirect".to_owned()),
            },
            security_schemes: Vec::new(),
            database: None,
            migrations: None,
            token_url: "/token".to_owned(),
        }
    }

    /// Адрес выдачи токена для кнопки Authorize в `/docs` (`tokenUrl` схемы `OAuth2PasswordBearer`,
    /// по умолчанию `/token`).
    pub fn oauth2_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    /// Подключает PostgreSQL по `DATABASE_URL` (пул ленивый: соединения открываются по запросу).
    /// Ошибка настройки (нет `DATABASE_URL`) вернётся из `run()`.
    pub fn database_from_env(mut self) -> Self {
        self.database = Some(db::pool_from_env().map(Db).map_err(|e| e.to_string()));
        self
    }

    /// Подключает готовый пул.
    pub fn database(mut self, pool: db::Pool) -> Self {
        self.database = Some(Ok(Db(pool)));
        self
    }

    /// Применять миграции из папки при старте (`App::run`/`serve`), например `"migrations"`.
    pub fn migrations(mut self, dir: impl Into<String>) -> Self {
        self.migrations = Some(dir.into());
        self
    }

    /// Пул базы, если она подключена.
    pub fn db(&self) -> Option<&Db> {
        self.database.as_ref().and_then(|d| d.as_ref().ok())
    }

    /// Применяет миграции (если заданы). Вызывается из `serve`; удобно и в тестах.
    pub async fn run_migrations(&self) -> Result<()> {
        let Some(dir) = &self.migrations else {
            return Ok(());
        };
        // Каталог есть, но пуст (новый проект без таблиц) — к базе не подключаемся.
        // Несуществующий каталог — ошибка ниже (опечатка в пути не должна проходить молча).
        let empty = std::fs::read_dir(dir).is_ok_and(|entries| {
            !entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|ext| ext == "sql"))
        });
        if empty {
            return Ok(());
        }
        let db = match &self.database {
            Some(Ok(db)) => db,
            Some(Err(e)) => return Err(Error::msg(e)),
            None => {
                return Err(Error::msg(
                    "миграции заданы, но база не подключена: добавьте .database_from_env() \
                     / migrations need a database",
                ));
            }
        };
        tracing::info!("Applying migrations from {dir:?}");
        db::migrate(db, dir).await?;
        Ok(())
    }

    /// Адрес схемы OpenAPI (по умолчанию `/openapi.json`).
    pub fn openapi_url(mut self, url: impl Into<String>) -> Self {
        self.docs.openapi_url = Some(url.into());
        self
    }

    /// Адрес Swagger UI (по умолчанию `/docs`).
    pub fn docs_url(mut self, url: impl Into<String>) -> Self {
        self.docs.docs_url = Some(url.into());
        self
    }

    /// Адрес ReDoc (по умолчанию `/redoc`).
    pub fn redoc_url(mut self, url: impl Into<String>) -> Self {
        self.docs.redoc_url = Some(url.into());
        self
    }

    /// Отключает `/openapi.json`, а с ним `/docs` и `/redoc` (как `openapi_url=None`).
    pub fn no_openapi(mut self) -> Self {
        self.docs.openapi_url = None;
        self
    }

    /// Отключает Swagger UI (как `docs_url=None`).
    pub fn no_docs(mut self) -> Self {
        self.docs.docs_url = None;
        self
    }

    /// Отключает ReDoc (как `redoc_url=None`).
    pub fn no_redoc(mut self) -> Self {
        self.docs.redoc_url = None;
        self
    }

    /// Добавляет схему безопасности в `components.securitySchemes`.
    pub fn security_scheme(mut self, scheme: SecurityScheme) -> Self {
        if !self.security_schemes.iter().any(|s| s.name == scheme.name) {
            self.security_schemes.push(scheme);
        }
        self
    }

    /// Документ OpenAPI 3.1 (то, что отдаёт `/openapi.json`).
    pub fn openapi(&self) -> Value {
        crate::openapi::build(
            &self.info,
            &self.router.routes(),
            &self.security_schemes,
            &self.token_url,
        )
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
        let openapi = self.openapi_json();
        let docs = self.docs.clone();
        let title = self.info.title.clone();
        let database = self.db().cloned();
        if let Some(Err(e)) = &self.database {
            tracing::error!("{e}");
        }
        let (mut router, mut matcher) = self.router.into_axum();
        if let (Some(openapi_url), Some(openapi)) = (&docs.openapi_url, openapi) {
            let openapi = Bytes::from(openapi);
            router = router.route(
                openapi_url,
                get(move || {
                    let body = openapi.clone();
                    async move { ([(header::CONTENT_TYPE, "application/json")], body) }
                }),
            );
            matcher.add(openapi_url);
            if let Some(docs_url) = &docs.docs_url {
                let html = crate::openapi::swagger_ui_html(
                    openapi_url,
                    &format!("{title} - Swagger UI"),
                    docs.oauth2_redirect_url.as_deref(),
                );
                router = router.route(docs_url, get(move || async move { Html(html) }));
                matcher.add(docs_url);
                if let Some(redirect) = &docs.oauth2_redirect_url {
                    router = router.route(
                        redirect,
                        get(|| async { Html(crate::openapi::swagger_ui_oauth2_redirect_html()) }),
                    );
                }
            }
            if let Some(redoc_url) = &docs.redoc_url {
                let html = crate::openapi::redoc_html(openapi_url, &format!("{title} - ReDoc"));
                router = router.route(redoc_url, get(move || async move { Html(html) }));
                matcher.add(redoc_url);
            }
        }
        if let Some(db) = database {
            router = router.layer(axum::Extension(db));
        }
        router
            .fallback(move |request: Request| not_found(matcher.clone(), request))
            .layer(CatchPanicLayer::custom(handle_panic))
            .layer(axum::middleware::from_fn(access_log))
    }

    /// Печатает таблицу маршрутов или OpenAPI в stdout (см. [`App::run`]).
    fn print(&self, what: &str) -> Result<()> {
        let value = match what {
            "routes" => Value::Array(
                self.routes()
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "method": r.method.as_str(),
                            "path": r.path,
                            "name": r.name,
                            "status_code": r.status_code.as_u16(),
                            "tags": r.tags,
                            "summary": r.summary,
                            "deprecated": r.deprecated,
                            "include_in_schema": r.include_in_schema,
                        })
                    })
                    .collect(),
            ),
            "openapi" => self.openapi(),
            other => {
                return Err(Error::msg(format!(
                    "RUSTYAPI_PRINT={other:?}: ожидается routes или openapi / expected routes or openapi"
                )));
            }
        };
        println!("{value}");
        Ok(())
    }

    /// Сериализованный OpenAPI (строится один раз при старте).
    fn openapi_json(&self) -> Option<String> {
        self.docs.openapi_url.as_ref()?;
        serde_json::to_string(&self.openapi()).ok()
    }

    /// Запускает сервер на `HOST:PORT` из переменных окружения (по умолчанию `127.0.0.1:8000`).
    /// Останавливается корректно по Ctrl+C / SIGTERM.
    ///
    /// С переменной `RUSTYAPI_PRINT=routes` или `RUSTYAPI_PRINT=openapi` сервер не запускается:
    /// в stdout печатается JSON таблицы маршрутов или схема OpenAPI (так работают `tez routes`
    /// и `tez generate client`).
    pub async fn run(self) -> Result<()> {
        if let Ok(what) = std::env::var("RUSTYAPI_PRINT") {
            return self.print(&what);
        }
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
        if let Some(Err(e)) = &self.database {
            return Err(Error::msg(e));
        }
        self.run_migrations().await?;
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
