//! Ответы 422 совпадают с FastAPI для тех же моделей.
//!
//! Эталон `fixtures/fastapi_422.json` снят с настоящего FastAPI скриптом `fixtures/fastapi_422.py`
//! (`pip install fastapi email-validator httpx && python3 fastapi_422.py fastapi_422.json`).

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
    #[serde(default)]
    tags: Vec<String>,
    description: Option<String>,
    #[field(url)]
    homepage: Option<String>,
    #[field(pattern = r"^[A-Z]{3}$")]
    code: Option<String>,
    #[field(default = 1, ge = 1, le = 1000)]
    quantity: i64,
    #[field(default = Status::Todo)]
    status: Status,
    #[field(default = Vec::new(), max_length = 3)]
    labels: Vec<Tag>,
}

#[post("/items")]
async fn create(Json(item): Json<ItemCreate>) -> Json<ItemCreate> {
    Json(item)
}

#[get("/items/{item_id}")]
async fn read(Path(_item_id): Path<i64>) -> Json<Value> {
    Json(json!({}))
}

#[schema]
struct SearchParams {
    q: String,
    #[field(default = 10)]
    limit: i64,
}

#[get("/search")]
async fn search(Query(_params): Query<SearchParams>) -> Json<Value> {
    Json(json!({}))
}

#[schema]
struct Login {
    username: String,
    password: String,
}

#[post("/login")]
async fn login(Form(_login): Form<Login>) -> Json<Value> {
    Json(json!({}))
}

fn app() -> App {
    App::new()
        .route(create)
        .route(read)
        .route(search)
        .route(login)
}

/// Тот же запрос, что в `fastapi_422.py`.
fn request(case: &str) -> Request<Body> {
    let ok = json!({"name": "x", "price": 1, "owner_email": "a@example.com"});
    let with = |key: &str, value: Value| {
        let mut body = ok.clone();
        body[key] = value;
        body
    };
    let json_body = match case {
        "missing" => Some(json!({"price": 1, "owner_email": "a@example.com"})),
        "short" => Some(with("name", json!(""))),
        "long" => Some(with("name", json!("a".repeat(101)))),
        "gt" => Some(with("price", json!(0))),
        "email" => Some(with("owner_email", json!("nope"))),
        "email2" => Some(with("owner_email", json!("a@b"))),
        "type_float" => Some(with("price", json!("abc"))),
        "type_str" => Some(with("name", json!(5))),
        "list_type" => Some(with("tags", json!("a"))),
        "list_item" => Some(with("tags", json!(["a", 1]))),
        "url" => Some(with("homepage", json!("notaurl"))),
        "url_scheme" => Some(with("homepage", json!("ftp://x.com"))),
        "pattern" => Some(with("code", json!("ab"))),
        "ge" => Some(with("quantity", json!(0))),
        "le" => Some(with("quantity", json!(1001))),
        "int_float" => Some(with("quantity", json!(1.5))),
        "int_str" => Some(with("quantity", json!("x"))),
        "enum" => Some(with("status", json!("zzz"))),
        "nested" => Some(with("labels", json!([{"name": ""}]))),
        "nested_missing" => Some(with("labels", json!([{}]))),
        "list_max" => Some(with(
            "labels",
            json!([{"name": "a"}, {"name": "a"}, {"name": "a"}, {"name": "a"}]),
        )),
        "not_object" => Some(json!([1])),
        _ => None,
    };
    if let Some(body) = json_body {
        return Request::post("/items")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
    }
    match case {
        "empty_body" => Request::post("/items").body(Body::empty()).unwrap(),
        "bad_json" => Request::post("/items")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name": "#))
            .unwrap(),
        "path_int" => Request::get("/items/abc").body(Body::empty()).unwrap(),
        "query_missing" => Request::get("/search").body(Body::empty()).unwrap(),
        "query_int" => Request::get("/search?q=a&limit=x")
            .body(Body::empty())
            .unwrap(),
        "form_missing" => Request::post("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("username=u"))
            .unwrap(),
        other => panic!("неизвестный кейс {other}"),
    }
}

/// Кейсы, где поведение сознательно отличается (см. docs/ru/02-schemas.md).
const DIFFERENT: &[&str] = &[
    // Pydantic в lax-режиме превращает `true` в `1.0`; RustyAPI строг, как serde.
    "type_float_bool",
];

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn matches_fastapi() {
    let expected: Value =
        rustyapi::serde_json::from_str(include_str!("fixtures/fastapi_422.json")).unwrap();
    let mut checked = 0;
    for (case, want) in expected.as_object().unwrap() {
        if DIFFERENT.contains(&case.as_str()) {
            continue;
        }
        let response = app().into_router().oneshot(request(case)).await.unwrap();
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = rustyapi::serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json!({"status": status, "body": body}),
            *want,
            "кейс `{case}` отличается от FastAPI"
        );
        checked += 1;
    }
    assert!(checked >= 25, "проверено только {checked} кейсов");
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn valid_body_roundtrip() {
    let body = json!({"name": "x", "price": 1.5, "owner_email": "a@example.com", "code": "ABC"});
    let response = app()
        .into_router()
        .oneshot(
            Request::post("/items")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = rustyapi::serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        json!({
            "name": "x", "price": 1.5, "owner_email": "a@example.com", "tags": [],
            "description": null, "homepage": null, "code": "ABC", "quantity": 1,
            "status": "todo", "labels": []
        })
    );
}
