//! Бенчмарк actix-web: валидация вручную.

use actix_web::{App, HttpResponse, HttpServer, get, post, web};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

#[get("/")]
async fn hello() -> HttpResponse {
    HttpResponse::Ok().json(json!({"message": "Hello, World!"}))
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

#[post("/items")]
async fn create_item(item: web::Json<Item>) -> HttpResponse {
    let item = item.into_inner();
    let len = item.name.chars().count();
    if len == 0 || len > 100 || item.price <= 0.0 {
        return HttpResponse::UnprocessableEntity().json(json!({"detail": "invalid"}));
    }
    HttpResponse::Ok().json(ItemOut {
        id: 1,
        name: item.name,
        price: item.price,
        tags: item.tags,
    })
}

#[derive(Serialize, sqlx::FromRow)]
struct Todo {
    id: i64,
    title: String,
    done: bool,
}

#[get("/todos/{id}")]
async fn read_todo(pool: web::Data<PgPool>, id: web::Path<i64>) -> HttpResponse {
    match sqlx::query_as::<_, Todo>("SELECT id, title, done FROM bench_todos WHERE id = $1")
        .bind(id.into_inner())
        .fetch_optional(pool.get_ref())
        .await
    {
        Ok(Some(todo)) => HttpResponse::Ok().json(todo),
        Ok(None) => HttpResponse::NotFound().json(json!({"detail": "Todo not found"})),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let size = std::env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);
    let pool = PgPoolOptions::new()
        .max_connections(size)
        .connect_lazy(&url)
        .unwrap();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8103);
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(hello)
            .service(create_item)
            .service(read_todo)
    })
    .bind(("127.0.0.1", port))?
    .run()
    .await
}
