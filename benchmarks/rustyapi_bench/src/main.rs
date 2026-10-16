//! Бенчмарк RustyAPI: те же три эндпоинта, что у axum, actix-web и FastAPI.

use rustyapi::prelude::*;

#[get("/")]
async fn hello() -> Json<Value> {
    Json(json!({"message": "Hello, World!"}))
}

#[schema]
struct Item {
    #[field(min_length = 1, max_length = 100)]
    name: String,
    #[field(gt = 0)]
    price: f64,
    #[field(default = Vec::new())]
    tags: Vec<String>,
}

#[schema]
struct ItemOut {
    id: i64,
    name: String,
    price: f64,
    tags: Vec<String>,
}

#[post("/items")]
async fn create_item(Json(item): Json<Item>) -> Json<ItemOut> {
    Json(ItemOut {
        id: 1,
        name: item.name,
        price: item.price,
        tags: item.tags,
    })
}

#[model(table = "bench_todos")]
struct Todo {
    #[primary_key]
    id: i64,
    title: String,
    done: bool,
}

#[get("/todos/{id}")]
async fn read_todo(db: Db, Path(id): Path<i64>) -> Result<Json<Todo>> {
    Ok(Json(Todo::get_or_404(&db, id).await?))
}

#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    App::new()
        .no_openapi()
        .route(hello)
        .route(create_item)
        .route(read_todo)
        .database_from_env()
        .run()
        .await
}
