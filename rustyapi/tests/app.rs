//! Поведение приложения: маршруты, ошибки в формате FastAPI, статусы, роутеры.

use http_body_util::BodyExt;
use rustyapi::axum::body::Body;
use rustyapi::axum::http::{Request, header};
use rustyapi::prelude::*;
use tower::ServiceExt;

#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({ "message": "Hello" }))
}

#[get("/items/{item_id}")]
async fn read_item(Path(item_id): Path<u32>) -> Result<Json<Value>> {
    if item_id == 0 {
        return Err(HTTPException::new(404, "Item not found").into());
    }
    Ok(Json(json!({ "item_id": item_id })))
}

#[post("/items", status_code = 201)]
async fn create_item(Json(body): Json<Value>) -> Json<Value> {
    Json(body)
}

#[delete("/items/{item_id}", status_code = status::HTTP_204_NO_CONTENT)]
async fn delete_item(Path(_item_id): Path<u32>) {}

#[get("/secret")]
async fn secret() -> Result<(), HTTPException> {
    Err(HTTPException::new(401, "Not authenticated").with_header("WWW-Authenticate", "Bearer"))
}

#[get("/teapot", status_code = 201)]
async fn teapot() -> HTTPException {
    HTTPException::new(418, json!({ "msg": "I'm a teapot", "loc": ["body"] }))
}

#[get("/panic")]
async fn boom() -> &'static str {
    panic!("boom")
}

#[get("/io")]
async fn io_error() -> Result<String> {
    Ok(std::fs::read_to_string("/definitely/not/here")?)
}

#[get("/files/{file_path:path}")]
async fn read_file(Path(file_path): Path<String>) -> String {
    file_path
}

/// Читает пользователя.
///
/// Вторая строка описания.
#[get("/{user_id}", tags = ["admin"], summary = "Get User", deprecated)]
async fn read_user(Path(user_id): Path<u32>) -> String {
    user_id.to_string()
}

fn app() -> App {
    let users = APIRouter::new()
        .prefix("/users")
        .tags(["users"])
        .route(read_user);
    App::new()
        .title("Test")
        .route(root)
        .route(read_item)
        .route(create_item)
        .route(delete_item)
        .route(secret)
        .route(teapot)
        .route(boom)
        .route(io_error)
        .route(read_file)
        .include_router(APIRouter::new().prefix("/api").include_router(users))
}

struct Reply {
    status: u16,
    headers: rustyapi::axum::http::HeaderMap,
    body: Value,
}

async fn call(method: &str, uri: &str, body: Option<Value>) -> Reply {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app()
        .into_router()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        rustyapi::serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Reply {
        status,
        headers,
        body,
    }
}

#[tokio::test]
async fn hello_world() {
    let r = call("GET", "/", None).await;
    assert_eq!((r.status, r.body), (200, json!({"message": "Hello"})));
}

#[tokio::test]
async fn unknown_path_is_fastapi_404() {
    let r = call("GET", "/nope", None).await;
    assert_eq!((r.status, r.body), (404, json!({"detail": "Not Found"})));
    assert_eq!(r.headers[header::CONTENT_TYPE], "application/json");
}

#[tokio::test]
async fn wrong_method_is_fastapi_405() {
    let r = call("PUT", "/items/1", None).await;
    assert_eq!(
        (r.status, r.body),
        (405, json!({"detail": "Method Not Allowed"}))
    );
    assert_eq!(r.headers[header::ALLOW], "GET,HEAD,DELETE");
}

#[tokio::test]
async fn http_exception() {
    let r = call("GET", "/items/0", None).await;
    assert_eq!(
        (r.status, r.body),
        (404, json!({"detail": "Item not found"}))
    );

    let r = call("GET", "/secret", None).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.body, json!({"detail": "Not authenticated"}));
    assert_eq!(r.headers[header::WWW_AUTHENTICATE], "Bearer");
}

#[tokio::test]
async fn http_exception_keeps_its_status_and_json_detail() {
    let r = call("GET", "/teapot", None).await;
    assert_eq!(r.status, 418);
    assert_eq!(
        r.body,
        json!({"detail": {"msg": "I'm a teapot", "loc": ["body"]}})
    );
}

#[tokio::test]
async fn status_code_parameter() {
    let r = call("POST", "/items", Some(json!({"name": "x"}))).await;
    assert_eq!((r.status, r.body), (201, json!({"name": "x"})));

    let r = call("DELETE", "/items/1", None).await;
    assert_eq!((r.status, r.body), (204, Value::Null));
}

#[tokio::test]
async fn panic_becomes_500_json() {
    let r = call("GET", "/panic", None).await;
    assert_eq!(
        (r.status, r.body),
        (500, json!({"detail": "Internal Server Error"}))
    );
}

#[tokio::test]
async fn any_error_becomes_500_json() {
    let r = call("GET", "/io", None).await;
    assert_eq!(
        (r.status, r.body),
        (500, json!({"detail": "Internal Server Error"}))
    );
}

#[tokio::test]
async fn path_converter() {
    let r = call("GET", "/files/a/b.txt", None).await;
    assert_eq!((r.status, r.body), (200, json!("a/b.txt")));
}

#[tokio::test]
async fn redirect_slashes() {
    let r = call("GET", "/items/5/?q=1", None).await;
    assert_eq!(r.status, 307);
    assert_eq!(r.headers[header::LOCATION], "/items/5?q=1");
}

#[tokio::test]
async fn nested_routers() {
    let r = call("GET", "/api/users/7", None).await;
    assert_eq!((r.status, r.body), (200, json!(7)));
}

#[test]
fn route_info() {
    let routes = app().routes();
    let user = routes.iter().find(|r| r.name == "read_user").unwrap();
    assert_eq!(user.method, "GET");
    assert_eq!(user.path, "/api/users/{user_id}");
    assert_eq!(user.tags, ["users", "admin"]);
    assert_eq!(user.summary.as_deref(), Some("Get User"));
    assert_eq!(
        user.description.as_deref(),
        Some("Читает пользователя.\n\nВторая строка описания.")
    );
    assert!(user.deprecated);

    let create = routes.iter().find(|r| r.name == "create_item").unwrap();
    assert_eq!(create.status_code, 201);
    assert!(!create.deprecated);
    assert_eq!(app().info().title, "Test");
}

#[test]
#[should_panic(expected = "route registered twice")]
fn duplicate_route_panics() {
    let _ = App::new().route(root).route(root).into_router();
}
