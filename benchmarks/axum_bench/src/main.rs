//! Бенчмарк «чистого» axum: валидация вручную, как пришлось бы писать без фреймворка.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

async fn hello() -> Json<Value> {
    Json(json!({"message": "Hello, World!"}))
}

#[derive(Deserialize)]
struct Item {
    name: String,
    price: f64,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Serialize)]
struct ItemOut {
    id: i64,
    name: String,
    price: f64,
    tags: Vec<String>,
}

async fn create_item(Json(item): Json<Item>) -> Response {
    let len = item.name.chars().count();
    if len == 0 || len > 100 || item.price <= 0.0 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "invalid"})),
        )
            .into_response();
    }
    Json(ItemOut {
        id: 1,
        name: item.name,
        price: item.price,
        tags: item.tags,
    })
    .into_response()
}

#[derive(Serialize, sqlx::FromRow)]
struct Todo {
    id: i64,
    title: String,
    done: bool,
}

async fn read_todo(State(pool): State<PgPool>, Path(id): Path<i64>) -> Response {
    match sqlx::query_as::<_, Todo>("SELECT id, title, done FROM bench_todos WHERE id = $1")
        .bind(id)
        .fetch_optional(&pool)
        .await
    {
        Ok(Some(todo)) => Json(todo).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "Todo not found"})),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[tokio::main]
async fn main() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let size = std::env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);
    let pool = PgPoolOptions::new()
        .max_connections(size)
        .connect_lazy(&url)
        .unwrap();
    let app = Router::new()
        .route("/", get(hello))
        .route("/items", post(create_item))
        .route("/todos/{id}", get(read_todo))
        .with_state(pool);
    let port = std::env::var("PORT").unwrap_or_else(|_| "8102".into());
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}
