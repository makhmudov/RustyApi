//! OpenAPI совпадает с FastAPI для того же приложения.
//!
//! Эталон `fixtures/fastapi_openapi.json` снят скриптом `fixtures/fastapi_openapi.py`.

use std::collections::HashMap;

use http_body_util::BodyExt;
use rustyapi::axum::body::Body;
use rustyapi::axum::http::Request;
use rustyapi::prelude::*;
use tower::ServiceExt;

#[schema]
#[serde(rename_all = "lowercase")]
enum Status {
    Todo,
    Done,
}

#[schema]
struct Tag {
    #[field(min_length = 1)]
    name: String,
}

#[schema]
struct ItemCreate {
    #[field(min_length = 1, max_length = 100)]
    name: String,
    #[field(gt = 0)]
    price: f64,
    #[field(email)]
    owner_email: String,
    #[field(default = Vec::new())]
    tags: Vec<String>,
    description: Option<String>,
    #[field(pattern = r"^[A-Z]{3}$")]
    code: Option<String>,
    #[field(default = Status::Todo)]
    status: Status,
    #[field(default = Vec::new(), max_length = 3)]
    labels: Vec<Tag>,
}

#[schema]
struct Item {
    id: i64,
    name: String,
    price: f64,
}

#[schema]
struct ListParams {
    q: String,
    #[field(default = 10)]
    limit: i64,
}

#[get("/")]
async fn list_items(Query(_params): Query<ListParams>) -> Json<Vec<Item>> {
    Json(vec![])
}

/// Читает товар по id.
#[get("/{item_id}")]
async fn read_item(Path(item_id): Path<i64>) -> Json<Item> {
    Json(Item {
        id: item_id,
        name: "x".into(),
        price: 1.0,
    })
}

#[post("/", status_code = 201)]
async fn create_item(Json(item): Json<ItemCreate>) -> Json<Item> {
    Json(Item {
        id: 1,
        name: item.name,
        price: item.price,
    })
}

#[delete("/{item_id}", status_code = 204, deprecated)]
async fn delete_item(Path(_item_id): Path<i64>) {}

#[get("/", summary = "Say Hello", tags = ["misc"])]
async fn root() -> Json<HashMap<String, Value>> {
    Json(HashMap::new())
}

#[schema]
struct Login {
    username: String,
    password: String,
}

#[post("/login")]
async fn login(Form(_login): Form<Login>) -> Json<String> {
    // `-> String` дал бы честный text/plain; FastAPI для `-> str` пишет application/json.
    Json(String::new())
}

#[get("/hidden", include_in_schema = false)]
async fn hidden() {}

fn app() -> App {
    let items = APIRouter::new()
        .prefix("/items")
        .tags(["items"])
        .route(list_items)
        .route(read_item)
        .route(create_item)
        .route(delete_item);
    App::new()
        .title("Parity")
        .version("1.2.3")
        .description("Сверка с FastAPI")
        .route(root)
        .route(login)
        .route(hidden)
        .include_router(items)
}

/// Числа сравниваются как f64 (`1` == `1.0`), ключи — без учёта порядка.
fn normalize(value: Value) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, normalize(v))).collect())
        }
        Value::String(s) => Value::String(
            // Форма в FastAPI называется по эндпоинту, у нас — по типу.
            s.replace("Body_login_login_post", "Login"),
        ),
        other => other,
    }
}

fn expected() -> Value {
    let text =
        include_str!("fixtures/fastapi_openapi.json").replace("Body_login_login_post", "Login");
    rustyapi::serde_json::from_str(&text).unwrap()
}

#[test]
fn paths_match_fastapi() {
    let ours = normalize(app().openapi());
    let theirs = normalize(expected());
    for (path, methods) in theirs["paths"].as_object().unwrap() {
        for (method, operation) in methods.as_object().unwrap() {
            assert_eq!(
                ours["paths"][path][method], *operation,
                "операция `{method} {path}` отличается от FastAPI"
            );
        }
    }
    assert_eq!(ours["paths"], theirs["paths"]);
    assert_eq!(ours["info"], theirs["info"]);
    assert_eq!(ours["openapi"], json!("3.1.0"));
}

#[test]
fn schemas_match_fastapi() {
    let ours = normalize(app().openapi());
    let theirs = normalize(expected());
    for (name, schema) in theirs["components"]["schemas"].as_object().unwrap() {
        assert_eq!(
            ours["components"]["schemas"][name], *schema,
            "схема `{name}` отличается от FastAPI"
        );
    }
    assert_eq!(
        ours["components"]["schemas"].as_object().unwrap().len(),
        theirs["components"]["schemas"].as_object().unwrap().len()
    );
}

async fn get(uri: &str) -> (u16, String, String) {
    let response = app()
        .into_router()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn docs_endpoints() {
    let (status, content_type, body) = get("/openapi.json").await;
    assert_eq!((status, content_type.as_str()), (200, "application/json"));
    let doc: Value = rustyapi::serde_json::from_str(&body).unwrap();
    assert_eq!(doc["info"]["title"], "Parity");

    let (status, content_type, body) = get("/docs").await;
    assert_eq!(status, 200);
    assert!(content_type.starts_with("text/html"));
    assert!(body.contains("swagger-ui-bundle.js") && body.contains("url: '/openapi.json'"));
    assert!(body.contains("<title>Parity - Swagger UI</title>"));

    let (status, _, body) = get("/redoc").await;
    assert_eq!(status, 200);
    assert!(body.contains(r#"<redoc spec-url="/openapi.json">"#));

    let (status, _, body) = get("/docs/oauth2-redirect").await;
    assert_eq!(status, 200);
    assert!(body.contains("oauth2"));
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn docs_can_be_disabled() {
    let app = App::new().route(root).no_docs().no_redoc();
    let response = app
        .into_router()
        .oneshot(Request::get("/docs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let response = App::new()
        .no_openapi()
        .into_router()
        .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}
