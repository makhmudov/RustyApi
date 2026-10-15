//! `APIRouter` и трейт `Endpoint`.
//!
//! Макросы `#[get("/path")]`, `#[post(...)]` и т.д. превращают функцию в unit-struct,
//! реализующий [`Endpoint`]. Регистрация: `router.route(read_item)`.

use std::collections::BTreeMap;

use axum::http::{Method, StatusCode};
use axum::routing::MethodRouter;

use crate::status::IntoStatusCode;

/// Описание маршрута: всё, что известно о нём без вызова обработчика (нужно для OpenAPI и `tez routes`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RouteInfo {
    /// HTTP-метод.
    pub method: Method,
    /// Полный путь в синтаксисе FastAPI: `/items/{item_id}`, `/files/{file_path:path}`.
    pub path: String,
    /// Имя функции-обработчика.
    pub name: &'static str,
    /// Краткое описание (`summary = "..."`).
    pub summary: Option<String>,
    /// Описание: `description = "..."` или doc-комментарий функции.
    pub description: Option<String>,
    /// Теги (сначала теги роутеров, затем теги эндпоинта, как в FastAPI).
    pub tags: Vec<String>,
    /// Помечен ли эндпоинт устаревшим.
    pub deprecated: bool,
    /// Статус успешного ответа (`status_code = 201`), по умолчанию 200.
    pub status_code: StatusCode,
}

impl RouteInfo {
    /// Новый маршрут со значениями по умолчанию.
    pub fn new(method: Method, path: impl Into<String>, name: &'static str) -> Self {
        Self {
            method,
            path: path.into(),
            name,
            summary: None,
            description: None,
            tags: Vec::new(),
            deprecated: false,
            status_code: StatusCode::OK,
        }
    }

    /// Задаёт `summary`.
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Задаёт `description`.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Добавляет теги.
    pub fn tags<I, T>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// Помечает эндпоинт устаревшим.
    pub fn deprecated(mut self, deprecated: bool) -> Self {
        self.deprecated = deprecated;
        self
    }

    /// Задаёт статус успешного ответа.
    pub fn status_code(mut self, status_code: impl IntoStatusCode) -> Self {
        self.status_code = status_code.into_status_code();
        self
    }
}

/// Эндпоинт: то, что генерирует `#[get]`/`#[post]`/... Обычно руками не реализуется.
pub trait Endpoint {
    /// Описание маршрута.
    fn info(&self) -> RouteInfo;
    /// Обработчик axum для этого маршрута.
    fn method_router(&self) -> MethodRouter;
}

/// Зарегистрированный маршрут: описание + обработчик.
#[derive(Debug)]
pub struct Route {
    info: RouteInfo,
    method_router: MethodRouter,
}

impl Route {
    /// Описание маршрута.
    pub fn info(&self) -> &RouteInfo {
        &self.info
    }
}

/// Группа маршрутов с общим префиксом и тегами — как `APIRouter` в FastAPI.
///
/// ```
/// use rustyapi::prelude::*;
///
/// #[get("/{item_id}")]
/// async fn read_item(Path(item_id): Path<u32>) -> Json<Value> {
///     Json(json!({ "item_id": item_id }))
/// }
///
/// let router = APIRouter::new().prefix("/items").tags(["items"]).route(read_item);
/// assert_eq!(router.routes()[0].path, "/items/{item_id}");
/// ```
#[derive(Debug, Default)]
pub struct APIRouter {
    prefix: String,
    tags: Vec<String>,
    routes: Vec<Route>,
}

impl APIRouter {
    /// Пустой роутер без префикса.
    pub fn new() -> Self {
        Self::default()
    }

    /// Префикс путей. Как в FastAPI: начинается с `/` и не заканчивается на `/`.
    ///
    /// # Panics
    /// Если префикс не начинается с `/` или заканчивается на `/`.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        validate_prefix(&prefix);
        self.prefix = prefix;
        self
    }

    /// Теги для всех маршрутов роутера.
    pub fn tags<I, T>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// Регистрирует эндпоинт (функцию под `#[get]`/`#[post]`/...).
    pub fn route(mut self, endpoint: impl Endpoint) -> Self {
        self.routes.push(Route {
            info: endpoint.info(),
            method_router: endpoint.method_router(),
        });
        self
    }

    /// Подключает другой роутер (аналог `router.include_router(other)`).
    pub fn include_router(mut self, router: APIRouter) -> Self {
        self.routes.extend(router.into_routes());
        self
    }

    /// Описания всех маршрутов с полными путями и тегами.
    pub fn routes(&self) -> Vec<RouteInfo> {
        self.routes
            .iter()
            .map(|route| self.resolve(route.info.clone()))
            .collect()
    }

    /// Маршруты с применёнными префиксом и тегами этого роутера.
    pub(crate) fn into_routes(mut self) -> Vec<Route> {
        let routes = std::mem::take(&mut self.routes);
        routes
            .into_iter()
            .map(|route| Route {
                info: self.resolve(route.info),
                method_router: route.method_router,
            })
            .collect()
    }

    fn resolve(&self, mut info: RouteInfo) -> RouteInfo {
        info.path = format!("{}{}", self.prefix, info.path);
        info.tags.splice(0..0, self.tags.iter().cloned());
        info
    }

    /// Собирает axum::Router. Маршруты на одном пути объединяются, неподходящий метод → 405 JSON.
    /// Вместе с роутером возвращает список путей — для редиректа `/items` ↔ `/items/`.
    pub(crate) fn into_axum(self) -> (axum::Router, PathMatcher) {
        let mut by_path: BTreeMap<String, MethodRouter> = BTreeMap::new();
        let mut seen: Vec<(Method, String)> = Vec::new();
        for route in self.into_routes() {
            let key = (route.info.method.clone(), route.info.path.clone());
            assert!(
                key.1.starts_with('/'),
                "путь маршрута `{}` должен начинаться с '/' (пустой путь допустим только под префиксом) \
                 / route path must start with '/'",
                route.info.name
            );
            assert!(
                !seen.contains(&key),
                "маршрут `{} {}` зарегистрирован дважды / route registered twice",
                key.0,
                key.1
            );
            seen.push(key);
            let path = to_axum_path(&route.info.path);
            let merged = match by_path.remove(&path) {
                Some(existing) => existing.merge(route.method_router),
                None => route.method_router,
            };
            by_path.insert(path, merged);
        }
        let matcher = PathMatcher {
            templates: by_path.keys().cloned().collect(),
        };
        let router =
            by_path
                .into_iter()
                .fold(axum::Router::new(), |router, (path, method_router)| {
                    router.route(&path, method_router.fallback(method_not_allowed))
                });
        (router, matcher)
    }
}

/// Проверяет, есть ли маршрут для пути (без вызова обработчиков). Нужен для `redirect_slashes`:
/// как в Starlette, `/items` → 307 на `/items/`, если зарегистрирован только второй вариант.
#[derive(Debug, Clone, Default)]
pub(crate) struct PathMatcher {
    templates: Vec<String>,
}

impl PathMatcher {
    /// Путь с добавленным или убранным завершающим `/`, если такой маршрут существует.
    pub(crate) fn redirect_target(&self, path: &str) -> Option<String> {
        let alternate = match path.strip_suffix('/') {
            Some("") => return None,
            Some(stripped) => stripped.to_owned(),
            None => format!("{path}/"),
        };
        self.templates
            .iter()
            .any(|template| path_matches(template, &alternate))
            .then_some(alternate)
    }
}

/// Сопоставление пути с шаблоном axum: `{x}` — один непустой сегмент, `{*x}` — остаток пути.
fn path_matches(template: &str, path: &str) -> bool {
    let mut template = template.split('/');
    let mut path = path.split('/');
    loop {
        match (template.next(), path.next()) {
            (None, None) => return true,
            (Some(t), Some(p)) => {
                if t.starts_with("{*") {
                    return !p.is_empty() || path.next().is_some();
                }
                let is_param = t.starts_with('{') && t.ends_with('}');
                if (is_param && p.is_empty()) || (!is_param && t != p) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

fn validate_prefix(prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    assert!(
        prefix.starts_with('/'),
        "префикс должен начинаться с '/' / a path prefix must start with '/': {prefix:?}"
    );
    assert!(
        !prefix.ends_with('/'),
        "префикс не должен заканчиваться на '/' / a path prefix must not end with '/': {prefix:?}"
    );
}

/// `{file_path:path}` (FastAPI/Starlette) → `{*file_path}` (axum). Остальное без изменений.
fn to_axum_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        let param = &rest[start + 1..start + len];
        match param.strip_suffix(":path") {
            Some(name) => {
                out.push_str("{*");
                out.push_str(name);
                out.push('}');
            }
            None => out.push_str(&rest[start..=start + len]),
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

async fn method_not_allowed() -> crate::HTTPException {
    crate::HTTPException::from_status(StatusCode::METHOD_NOT_ALLOWED)
}

/// Не публичный API: используется кодом, который генерируют макросы.
#[doc(hidden)]
pub mod __private {
    pub use axum::http::Method;
    pub use axum::routing::{MethodFilter, MethodRouter};

    use axum::handler::Handler;
    use axum::http::StatusCode;
    use axum::response::Response;

    pub use crate::status::IntoStatusCode;

    /// Обработчик для метода; если задан `status_code`, успешный ответ 200 получает этот статус.
    pub fn on<H, T>(filter: MethodFilter, handler: H, status: Option<StatusCode>) -> MethodRouter
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let method_router = axum::routing::on(filter, handler);
        match status {
            Some(status) if status != StatusCode::OK => method_router.route_layer(
                axum::middleware::map_response(move |mut response: Response| async move {
                    if response.status() == StatusCode::OK {
                        *response.status_mut() = status;
                    }
                    response
                }),
            ),
            _ => method_router,
        }
    }

    /// Статус из выражения в `status_code = ...`.
    pub fn status(code: impl IntoStatusCode) -> StatusCode {
        code.into_status_code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_path_converter() {
        assert_eq!(
            to_axum_path("/files/{file_path:path}"),
            "/files/{*file_path}"
        );
        assert_eq!(to_axum_path("/items/{id}/x"), "/items/{id}/x");
        assert_eq!(to_axum_path("/"), "/");
    }

    #[test]
    fn redirect_slashes() {
        let matcher = PathMatcher {
            templates: vec![
                "/items/".into(),
                "/items/{id}".into(),
                "/files/{*rest}".into(),
            ],
        };
        assert_eq!(
            matcher.redirect_target("/items").as_deref(),
            Some("/items/")
        );
        assert_eq!(
            matcher.redirect_target("/items/5/").as_deref(),
            Some("/items/5")
        );
        assert_eq!(
            matcher.redirect_target("/files/a/b/").as_deref(),
            Some("/files/a/b")
        );
        assert_eq!(matcher.redirect_target("/items/5"), None);
        assert_eq!(matcher.redirect_target("/"), None);
        assert_eq!(matcher.redirect_target("/other"), None);
    }

    #[test]
    #[should_panic(expected = "must not end with '/'")]
    fn rejects_trailing_slash_prefix() {
        let _ = APIRouter::new().prefix("/items/");
    }
}
