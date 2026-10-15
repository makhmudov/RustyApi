//! Генератор OpenAPI 3.1, Swagger UI (`/docs`) и ReDoc (`/redoc`) — как в FastAPI.
//!
//! Схема строится из [`RouteInfo`]: аргументы и возвращаемый тип обработчика описываются кодом,
//! который генерирует `#[get]`/`#[post]`/... через autoref-специализацию: типы, реализующие
//! [`OpenApiArg`] / [`OpenApiResponse`], попадают в схему, остальные молча пропускаются.

use axum::http::StatusCode;
use serde_json::{Map, Value, json};

use crate::app::AppInfo;
use crate::extract::{Form, Json, Path, Query};
use crate::routing::RouteInfo;
use crate::schema::{ApiSchema, Components, title_case};

/// Операция OpenAPI в процессе построения.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct Operation {
    /// Имена параметров пути из шаблона (`/items/{item_id}` → `item_id`).
    pub path_params: Vec<String>,
    /// `parameters`.
    pub parameters: Vec<Value>,
    /// `requestBody`.
    pub request_body: Option<Value>,
    /// Тело успешного ответа: (content-type, схема). `None` — тип неизвестен (`{}` как в FastAPI).
    pub response: Option<ResponseBody>,
    /// Требования безопасности (`security`).
    pub security: Vec<Value>,
    /// Дополнительные ответы (`"401": {...}`).
    pub extra_responses: Map<String, Value>,
}

/// Тело успешного ответа.
#[derive(Debug, Clone)]
pub enum ResponseBody {
    /// Ответ без тела (`()`, 204).
    Empty,
    /// Тело с content-type и схемой.
    Content(&'static str, Value),
}

impl Operation {
    /// Добавляет параметр (`in`: `path`, `query`, `header`, `cookie`), если такого ещё нет.
    pub fn add_parameter(&mut self, name: &str, location: &str, required: bool, schema: Value) {
        let exists = self
            .parameters
            .iter()
            .any(|p| p["name"] == name && p["in"] == location);
        if exists {
            return;
        }
        let mut param = Map::new();
        param.insert("name".into(), json!(name));
        param.insert("in".into(), json!(location));
        param.insert("required".into(), json!(required));
        if let Some(description) = schema.get("description") {
            param.insert("description".into(), description.clone());
        }
        param.insert("schema".into(), schema);
        self.parameters.push(Value::Object(param));
    }
}

/// Аргумент обработчика, который описывает себя в OpenAPI (`Json<T>`, `Path<T>`, `Query<T>`, `Form<T>`).
pub trait OpenApiArg {
    /// Дополняет операцию.
    fn doc_arg(operation: &mut Operation, components: &mut Components);
}

/// Возвращаемый тип обработчика, который описывает себя в OpenAPI.
pub trait OpenApiResponse {
    /// Дополняет операцию.
    fn doc_response(operation: &mut Operation, components: &mut Components);
}

fn with_title(mut schema: Value, name: &str) -> Value {
    if let Value::Object(map) = &mut schema {
        if !map.contains_key("$ref") && !map.contains_key("title") {
            map.insert("title".into(), json!(title_case(name)));
        }
    }
    schema
}

impl<T: ApiSchema> OpenApiArg for Json<T> {
    fn doc_arg(operation: &mut Operation, components: &mut Components) {
        let schema = T::schema(components);
        operation.request_body = Some(json!({
            "content": {"application/json": {"schema": schema}},
            "required": T::required(),
        }));
    }
}

impl<T: ApiSchema> OpenApiArg for Form<T> {
    fn doc_arg(operation: &mut Operation, components: &mut Components) {
        let schema = T::schema(components);
        operation.request_body = Some(json!({
            "content": {"application/x-www-form-urlencoded": {"schema": schema}},
            "required": true,
        }));
    }
}

/// Поля объектной схемы: (имя, схема поля, обязательно ли).
fn object_fields(schema: &Value, components: &Components) -> Option<Vec<(String, Value, bool)>> {
    let resolved = components.resolve(schema);
    let properties = resolved.get("properties")?.as_object()?;
    let required: Vec<&str> = resolved
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Some(
        properties
            .iter()
            .map(|(name, prop)| {
                (
                    name.clone(),
                    prop.clone(),
                    required.contains(&name.as_str()),
                )
            })
            .collect(),
    )
}

impl<T: ApiSchema> OpenApiArg for Query<T> {
    fn doc_arg(operation: &mut Operation, components: &mut Components) {
        // Схема не должна попасть в components: параметры раскладываются по одному, как в FastAPI.
        let mut scratch = components.clone();
        let schema = T::schema(&mut scratch);
        let Some(fields) = object_fields(&schema, &scratch) else {
            return;
        };
        // Вложенные типы (enum в параметре) нужны в components.
        for (name, value) in scratch.schemas() {
            if components.get(name).is_none() && Some(name) != T::schema_name().as_ref() {
                components.insert(name.clone(), value.clone());
            }
        }
        for (name, prop, required) in fields {
            operation.add_parameter(&name, "query", required, prop);
        }
    }
}

impl<T: ApiSchema> OpenApiArg for Path<T> {
    fn doc_arg(operation: &mut Operation, components: &mut Components) {
        let mut scratch = components.clone();
        let schema = T::schema(&mut scratch);
        let names = operation.path_params.clone();
        if let Some(fields) = object_fields(&schema, &scratch) {
            for (name, prop, _) in fields {
                operation.add_parameter(&name, "path", true, prop);
            }
        } else if let Some(items) = schema.get("prefixItems").and_then(Value::as_array) {
            for (name, item) in names.iter().zip(items) {
                operation.add_parameter(name, "path", true, with_title(item.clone(), name));
            }
        } else if let Some(name) = names.first() {
            let schema = T::schema(components);
            operation.add_parameter(name, "path", true, with_title(schema, name));
        }
    }
}

impl<T: ApiSchema> OpenApiResponse for Json<T> {
    fn doc_response(operation: &mut Operation, components: &mut Components) {
        operation.response = Some(ResponseBody::Content(
            "application/json",
            T::schema(components),
        ));
    }
}

impl<T: OpenApiResponse, E> OpenApiResponse for Result<T, E> {
    fn doc_response(operation: &mut Operation, components: &mut Components) {
        T::doc_response(operation, components);
    }
}

impl<T: OpenApiResponse> OpenApiResponse for (StatusCode, T) {
    fn doc_response(operation: &mut Operation, components: &mut Components) {
        T::doc_response(operation, components);
    }
}

impl OpenApiResponse for () {
    fn doc_response(operation: &mut Operation, _: &mut Components) {
        operation.response = Some(ResponseBody::Empty);
    }
}

impl OpenApiResponse for StatusCode {
    fn doc_response(operation: &mut Operation, _: &mut Components) {
        operation.response = Some(ResponseBody::Empty);
    }
}

macro_rules! text_response {
    ($($ty:ty => $mime:literal),*) => {
        $(
            impl OpenApiResponse for $ty {
                fn doc_response(operation: &mut Operation, _: &mut Components) {
                    operation.response =
                        Some(ResponseBody::Content($mime, json!({"type": "string"})));
                }
            }
        )*
    };
}

text_response!(String => "text/plain", &'static str => "text/plain");

impl<T> OpenApiResponse for axum::response::Html<T> {
    fn doc_response(operation: &mut Operation, _: &mut Components) {
        operation.response = Some(ResponseBody::Content(
            "text/html",
            json!({"type": "string"}),
        ));
    }
}

/// `{"type": "...", "loc": [...], ...}` — как `ValidationError` в FastAPI.
fn validation_error_schemas(components: &mut Components) {
    if components.get("HTTPValidationError").is_some() {
        return;
    }
    components.insert(
        "ValidationError",
        json!({
            "properties": {
                "loc": {
                    "items": {"anyOf": [{"type": "string"}, {"type": "integer"}]},
                    "type": "array",
                    "title": "Location"
                },
                "msg": {"type": "string", "title": "Message"},
                "type": {"type": "string", "title": "Error Type"},
                "input": {"title": "Input"},
                "ctx": {"type": "object", "title": "Context"}
            },
            "type": "object",
            "required": ["loc", "msg", "type"],
            "title": "ValidationError"
        }),
    );
    components.insert(
        "HTTPValidationError",
        json!({
            "properties": {
                "detail": {
                    "items": {"$ref": "#/components/schemas/ValidationError"},
                    "type": "array",
                    "title": "Detail"
                }
            },
            "type": "object",
            "title": "HTTPValidationError"
        }),
    );
}

/// Параметры пути из шаблона: `/files/{file_path:path}` → `["file_path"]`.
fn template_params(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        let param = &rest[start + 1..start + len];
        out.push(param.split(':').next().unwrap_or(param).to_owned());
        rest = &rest[start + len + 1..];
    }
    out
}

/// Путь для OpenAPI: конвертер `:path` убирается (`/files/{file_path}`), как в FastAPI.
fn openapi_path(path: &str) -> String {
    let mut out = path.to_owned();
    for name in template_params(path) {
        out = out.replace(&format!("{{{name}:path}}"), &format!("{{{name}}}"));
    }
    out
}

/// `operationId` как `generate_unique_id` в FastAPI: `read_item_items__item_id__get`.
pub fn operation_id(name: &str, path: &str, method: &str) -> String {
    let raw = format!("{name}{}", openapi_path(path));
    let mut id: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    id.push('_');
    id.push_str(&method.to_ascii_lowercase());
    id
}

/// `str.title().replace("_", " ")` из Python (так Pydantic строит заголовки).
fn python_title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_letter = false;
    for c in text.chars() {
        if c.is_alphabetic() {
            if prev_letter {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            prev_letter = true;
        } else {
            out.push(if c == '_' { ' ' } else { c });
            prev_letter = false;
        }
    }
    out
}

/// Схема безопасности для `components.securitySchemes`.
#[derive(Debug, Clone)]
pub struct SecurityScheme {
    /// Имя схемы (`OAuth2PasswordBearer`).
    pub name: String,
    /// Определение.
    pub scheme: Value,
}

/// Собирает документ OpenAPI 3.1.
pub fn build(
    info: &AppInfo,
    routes: &[RouteInfo],
    security_schemes: &[SecurityScheme],
    token_url: &str,
) -> Value {
    let mut security_schemes = security_schemes.to_vec();
    let mut components = Components::new();
    let mut paths: Map<String, Value> = Map::new();
    let mut needs_validation = false;

    for route in routes.iter().filter(|r| r.include_in_schema) {
        let mut op = Operation {
            path_params: template_params(&route.path),
            ..Operation::default()
        };
        if let Some(doc) = route.openapi {
            doc(&mut op, &mut components);
        }
        // Параметры пути, не описанные экстрактором, — строки (FastAPI их бы не принял).
        for name in op.path_params.clone() {
            op.add_parameter(
                &name,
                "path",
                true,
                json!({"type": "string", "title": title_case(&name)}),
            );
        }

        let method = route.method.as_str().to_ascii_lowercase();
        let mut operation = Map::new();
        if !route.tags.is_empty() {
            operation.insert("tags".into(), json!(route.tags));
        }
        let summary = route
            .summary
            .clone()
            .unwrap_or_else(|| title_case(route.name));
        operation.insert("summary".into(), json!(summary));
        if let Some(description) = &route.description {
            operation.insert("description".into(), json!(description));
        }
        operation.insert(
            "operationId".into(),
            json!(operation_id(route.name, &route.path, route.method.as_str())),
        );
        if route.deprecated {
            operation.insert("deprecated".into(), json!(true));
        }
        let has_input = !op.parameters.is_empty() || op.request_body.is_some();
        if !op.parameters.is_empty() {
            operation.insert("parameters".into(), Value::Array(op.parameters));
        }
        if let Some(body) = op.request_body {
            operation.insert("requestBody".into(), body);
        }

        let mut responses = Map::new();
        let code = route.status_code.as_u16();
        let mut success = Map::new();
        success.insert("description".into(), json!("Successful Response"));
        let no_body = matches!(code, 204 | 304) || code < 200;
        match op.response {
            _ if no_body => {}
            Some(ResponseBody::Empty) => {}
            Some(ResponseBody::Content(mime, mut schema)) => {
                // Встроенная схема ответа получает заголовок `Response Read Item Items  Get`.
                if let Value::Object(map) = &mut schema {
                    if !map.contains_key("$ref") && !map.is_empty() {
                        let id = operation_id(route.name, &route.path, route.method.as_str());
                        map.insert(
                            "title".into(),
                            json!(python_title(&format!("Response_{id}"))),
                        );
                    }
                }
                success.insert("content".into(), json!({ mime: {"schema": schema} }));
            }
            None => {
                success.insert(
                    "content".into(),
                    json!({"application/json": {"schema": {}}}),
                );
            }
        }
        responses.insert(code.to_string(), Value::Object(success));
        for (code, response) in op.extra_responses {
            responses.insert(code, response);
        }
        if has_input {
            needs_validation = true;
            responses.insert(
                "422".into(),
                json!({
                    "description": "Validation Error",
                    "content": {"application/json": {"schema": {"$ref": "#/components/schemas/HTTPValidationError"}}}
                }),
            );
        }
        operation.insert("responses".into(), Value::Object(responses));
        if !op.security.is_empty() {
            let uses_bearer = op
                .security
                .iter()
                .any(|s| s.get(crate::auth::SECURITY_SCHEME).is_some());
            if uses_bearer
                && !security_schemes
                    .iter()
                    .any(|s| s.name == crate::auth::SECURITY_SCHEME)
            {
                security_schemes.push(crate::auth::security_scheme(token_url));
            }
            operation.insert("security".into(), Value::Array(op.security));
        }

        let entry = paths
            .entry(openapi_path(&route.path))
            .or_insert_with(|| json!({}));
        entry[method] = Value::Object(operation);
    }

    if needs_validation {
        validation_error_schemas(&mut components);
    }

    let mut info_obj = Map::new();
    info_obj.insert("title".into(), json!(info.title));
    if let Some(description) = &info.description {
        info_obj.insert("description".into(), json!(description));
    }
    info_obj.insert("version".into(), json!(info.version));

    let mut doc = Map::new();
    doc.insert("openapi".into(), json!("3.1.0"));
    doc.insert("info".into(), Value::Object(info_obj));
    doc.insert("paths".into(), Value::Object(paths));
    let schemas = components.into_schemas();
    let mut components_obj = Map::new();
    if !schemas.is_empty() {
        components_obj.insert(
            "schemas".into(),
            Value::Object(schemas.into_iter().collect()),
        );
    }
    if !security_schemes.is_empty() {
        components_obj.insert(
            "securitySchemes".into(),
            Value::Object(
                security_schemes
                    .iter()
                    .map(|s| (s.name.clone(), s.scheme.clone()))
                    .collect(),
            ),
        );
    }
    if !components_obj.is_empty() {
        doc.insert("components".into(), Value::Object(components_obj));
    }
    Value::Object(doc)
}

/// HTML Swagger UI (как `get_swagger_ui_html` в FastAPI, ресурсы с CDN jsdelivr).
pub fn swagger_ui_html(
    openapi_url: &str,
    title: &str,
    oauth2_redirect_url: Option<&str>,
) -> String {
    let title = html_escape(title);
    let redirect = oauth2_redirect_url
        .map(|url| format!("oauth2RedirectUrl: window.location.origin + '{url}',"))
        .unwrap_or_default();
    format!(
        r##"<!DOCTYPE html>
<html>
<head>
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link type="text/css" rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
<link rel="shortcut icon" href="https://fastapi.tiangolo.com/img/favicon.png">
<title>{title}</title>
</head>
<body>
<div id="swagger-ui">
</div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>
const ui = SwaggerUIBundle({{
    url: '{openapi_url}',
    "dom_id": "#swagger-ui",
    "layout": "BaseLayout",
    "deepLinking": true,
    "showExtensions": true,
    "showCommonExtensions": true,
    {redirect}
    presets: [
        SwaggerUIBundle.presets.apis,
        SwaggerUIBundle.SwaggerUIStandalonePreset
    ],
}})
</script>
</body>
</html>
"##
    )
}

/// HTML ReDoc (как `get_redoc_html` в FastAPI).
pub fn redoc_html(openapi_url: &str, title: &str) -> String {
    let title = html_escape(title);
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<title>{title}</title>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link href="https://fonts.googleapis.com/css?family=Montserrat:300,400,700|Roboto:300,400,700" rel="stylesheet">
<link rel="shortcut icon" href="https://fastapi.tiangolo.com/img/favicon.png">
<style>
  body {{
    margin: 0;
    padding: 0;
  }}
</style>
</head>
<body>
<noscript>
    ReDoc requires Javascript to function. Please enable it to browse the documentation.
</noscript>
<redoc spec-url="{openapi_url}"></redoc>
<script src="https://cdn.jsdelivr.net/npm/redoc@2/bundles/redoc.standalone.js"> </script>
</body>
</html>
"#
    )
}

/// Страница возврата OAuth2 для Swagger UI (как `get_swagger_ui_oauth2_redirect_html`).
pub fn swagger_ui_oauth2_redirect_html() -> &'static str {
    include_str!("oauth2-redirect.html")
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Не публичный API: используется кодом, который генерируют макросы маршрутов.
#[doc(hidden)]
pub mod __private {
    use std::marker::PhantomData;

    pub use super::{OpenApiArg, OpenApiResponse, Operation};
    pub use crate::schema::Components;

    pub struct Probe<T: ?Sized>(PhantomData<T>);

    impl<T: ?Sized> Probe<T> {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self(PhantomData)
        }
    }

    pub trait ViaArg {
        fn __rustyapi_doc_arg(&self, operation: &mut Operation, components: &mut Components);
    }

    impl<T: OpenApiArg + ?Sized> ViaArg for &Probe<T> {
        fn __rustyapi_doc_arg(&self, operation: &mut Operation, components: &mut Components) {
            T::doc_arg(operation, components);
        }
    }

    pub trait NoArg {
        fn __rustyapi_doc_arg(&self, _: &mut Operation, _: &mut Components) {}
    }

    impl<T: ?Sized> NoArg for Probe<T> {}

    pub trait ViaResponse {
        fn __rustyapi_doc_response(&self, operation: &mut Operation, components: &mut Components);
    }

    impl<T: OpenApiResponse + ?Sized> ViaResponse for &Probe<T> {
        fn __rustyapi_doc_response(&self, operation: &mut Operation, components: &mut Components) {
            T::doc_response(operation, components);
        }
    }

    pub trait NoResponse {
        fn __rustyapi_doc_response(&self, _: &mut Operation, _: &mut Components) {}
    }

    impl<T: ?Sized> NoResponse for Probe<T> {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids() {
        assert_eq!(
            operation_id("read_item", "/items/{item_id}", "GET"),
            "read_item_items__item_id__get"
        );
        assert_eq!(operation_id("root", "/", "GET"), "root__get");
        assert_eq!(
            operation_id("read_file", "/files/{file_path:path}", "GET"),
            "read_file_files__file_path__get"
        );
    }

    #[test]
    fn titles() {
        assert_eq!(python_title("Response_root__get"), "Response Root  Get");
    }

    #[test]
    fn paths() {
        assert_eq!(
            openapi_path("/files/{file_path:path}"),
            "/files/{file_path}"
        );
        assert_eq!(template_params("/a/{x}/b/{y:path}"), ["x", "y"]);
    }
}
