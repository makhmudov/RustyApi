//! `TestClient` — как `fastapi.testclient.TestClient`: запросы к приложению без сети.
//!
//! ```
//! use rustyapi::prelude::*;
//! use rustyapi::testing::TestClient;
//!
//! #[post("/items", status_code = 201)]
//! async fn create(Json(body): Json<Value>) -> Json<Value> {
//!     Json(body)
//! }
//!
//! # #[rustyapi::tokio::main(crate = "rustyapi::tokio")]
//! # async fn main() {
//! let client = TestClient::new(App::new().route(create));
//! let response = client.post("/items").json(&json!({"name": "x"})).send().await;
//! assert_eq!(response.status(), 201);
//! assert_eq!(response.json::<Value>(), json!({"name": "x"}));
//! # }
//! ```

use std::future::{Future, IntoFuture};
use std::net::SocketAddr;
use std::pin::Pin;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

use crate::App;

/// Клиент для тестов. Каждый запрос проходит через все слои приложения (логи, CORS, лимиты...).
#[derive(Clone)]
pub struct TestClient {
    router: Router,
}

impl std::fmt::Debug for TestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestClient").finish_non_exhaustive()
    }
}

impl TestClient {
    /// Клиент для приложения.
    pub fn new(app: App) -> Self {
        Self {
            router: app.into_router(),
        }
    }

    /// Клиент для готового axum-роутера.
    pub fn from_router(router: Router) -> Self {
        Self { router }
    }

    /// Запрос с произвольным методом.
    pub fn request(&self, method: Method, uri: impl Into<String>) -> TestRequest {
        TestRequest {
            router: self.router.clone(),
            method,
            uri: uri.into(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
        }
    }

    /// `GET`.
    pub fn get(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::GET, uri)
    }

    /// `POST`.
    pub fn post(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::POST, uri)
    }

    /// `PUT`.
    pub fn put(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::PUT, uri)
    }

    /// `PATCH`.
    pub fn patch(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::PATCH, uri)
    }

    /// `DELETE`.
    pub fn delete(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::DELETE, uri)
    }

    /// `OPTIONS`.
    pub fn options(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::OPTIONS, uri)
    }
}

/// Построитель запроса. Отправка — `.send().await` или просто `.await`.
#[must_use = "запрос не отправлен: добавьте .send().await"]
pub struct TestRequest {
    router: Router,
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Bytes,
}

impl TestRequest {
    /// Заголовок.
    ///
    /// # Panics
    /// Если имя или значение заголовка некорректны.
    pub fn header(mut self, name: &str, value: impl AsRef<str>) -> Self {
        let name = HeaderName::try_from(name).expect("некорректное имя заголовка");
        let value = HeaderValue::try_from(value.as_ref()).expect("некорректное значение заголовка");
        self.headers.append(name, value);
        self
    }

    /// `Authorization: Bearer <token>`.
    pub fn bearer(self, token: impl AsRef<str>) -> Self {
        let value = format!("Bearer {}", token.as_ref());
        self.header("authorization", value)
    }

    /// JSON-тело (`content-type: application/json`).
    pub fn json(mut self, body: &impl Serialize) -> Self {
        self.body = serde_json::to_vec(body)
            .expect("тело не сериализуется в JSON")
            .into();
        self.set_content_type("application/json")
    }

    /// Тело формы (`application/x-www-form-urlencoded`).
    pub fn form(mut self, body: &impl Serialize) -> Self {
        self.body = serde_urlencoded::to_string(body)
            .expect("тело не сериализуется в форму")
            .into();
        self.set_content_type("application/x-www-form-urlencoded")
    }

    /// `multipart/form-data` (загрузка файлов).
    pub fn multipart(mut self, form: MultipartBody) -> Self {
        let (content_type, body) = form.finish();
        self.body = body.into();
        self.set_content_type(&content_type)
    }

    /// Сырое тело.
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Параметры строки запроса.
    pub fn query(mut self, query: &impl Serialize) -> Self {
        let encoded = serde_urlencoded::to_string(query).expect("параметры не сериализуются");
        if !encoded.is_empty() {
            self.uri
                .push(if self.uri.contains('?') { '&' } else { '?' });
            self.uri.push_str(&encoded);
        }
        self
    }

    fn set_content_type(mut self, value: &str) -> Self {
        self.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::try_from(value).expect("content-type"),
        );
        self
    }

    /// Отправляет запрос.
    pub async fn send(self) -> TestResponse {
        let mut request = Request::builder()
            .method(self.method)
            .uri(&self.uri)
            .body(Body::from(self.body))
            .unwrap_or_else(|e| panic!("некорректный запрос {}: {e}", self.uri));
        *request.headers_mut() = self.headers;
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
        let response = self
            .router
            .oneshot(request)
            .await
            .unwrap_or_else(|never| match never {});
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("не удалось прочитать тело ответа");
        TestResponse {
            status,
            headers,
            body,
        }
    }
}

impl IntoFuture for TestRequest {
    type Output = TestResponse;
    type IntoFuture = Pin<Box<dyn Future<Output = TestResponse> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.send())
    }
}

/// Ответ: статус, заголовки, тело.
#[derive(Debug, Clone)]
pub struct TestResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl TestResponse {
    /// Статус (`response.status() == 201` работает).
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Статус числом, как `response.status_code` в FastAPI.
    pub fn status_code(&self) -> u16 {
        self.status.as_u16()
    }

    /// Заголовки.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Значение заголовка.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Тело как текст.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Тело как байты.
    pub fn bytes(&self) -> &Bytes {
        &self.body
    }

    /// Тело как JSON: `response.json::<Value>()` или своя структура.
    ///
    /// # Panics
    /// Если тело — не JSON нужной формы (сообщение содержит статус и тело).
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "ответ {} — не JSON нужной формы ({e}): {}",
                self.status,
                self.text()
            )
        })
    }
}

/// Тело `multipart/form-data` для [`TestRequest::multipart`].
#[derive(Debug, Clone, Default)]
pub struct MultipartBody {
    parts: Vec<Part>,
}

/// Поле формы: имя, (имя файла, MIME) для файлов, содержимое.
type Part = (String, Option<(String, String)>, Vec<u8>);

impl MultipartBody {
    /// Пустая форма.
    pub fn new() -> Self {
        Self::default()
    }

    /// Текстовое поле.
    pub fn text(mut self, name: &str, value: impl Into<String>) -> Self {
        self.parts
            .push((name.to_owned(), None, value.into().into_bytes()));
        self
    }

    /// Файл.
    pub fn file(
        mut self,
        name: &str,
        filename: &str,
        content_type: &str,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        self.parts.push((
            name.to_owned(),
            Some((filename.to_owned(), content_type.to_owned())),
            data.into(),
        ));
        self
    }

    fn finish(self) -> (String, Vec<u8>) {
        let boundary = format!("rustyapi-boundary-{}", uuid::Uuid::new_v4().simple());
        let mut body = Vec::new();
        for (name, file, data) in self.parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match file {
                Some((filename, content_type)) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n\
                         Content-Type: {content_type}\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(&data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }
}
