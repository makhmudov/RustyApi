//! Эталонный пример RustyAPI (JWT + PostgreSQL + CRUD). Наполняется по мере этапов ROADMAP.md.
//!
//! Этап 1: роуты, `APIRouter`, `HTTPException`, `status_code`.
//! Этап 2: схемы `#[schema]`, валидация `#[field(...)]`, ошибки 422 как в FastAPI.
//! Этап 3: OpenAPI, `/docs`, `/redoc`.
//! Этап 4: PostgreSQL, `#[model]`, миграции при старте.
//!
//! ```text
//! cd examples/todo_api
//! docker compose up -d
//! cp .env.example .env
//! cargo run
//! curl -X POST http://127.0.0.1:8000/todos -H 'content-type: application/json' -d '{"title":"Купить хлеб"}'
//! open http://127.0.0.1:8000/docs
//! ```

use rustyapi::chrono::{DateTime, Utc};
use rustyapi::prelude::*;

/// Задача.
#[model(table = "todos")]
struct Todo {
    #[primary_key]
    id: i64,
    title: String,
    done: bool,
    created_at: DateTime<Utc>,
}

/// Новая задача.
#[schema]
struct TodoCreate {
    /// Заголовок задачи.
    #[field(min_length = 1, max_length = 200, example = "Купить хлеб")]
    title: String,
}

/// Изменение задачи: передаются только меняющиеся поля.
#[schema]
struct TodoUpdate {
    #[field(min_length = 1, max_length = 200)]
    title: Option<String>,
    done: Option<bool>,
}

/// Фильтр списка задач.
#[schema]
struct TodoFilter {
    /// Только выполненные / только невыполненные.
    done: Option<bool>,
    /// Сколько задач вернуть.
    #[field(default = 100, ge = 1, le = 1000)]
    limit: i64,
    /// Сколько задач пропустить.
    #[field(default = 0, ge = 0)]
    offset: i64,
}

/// Приветствие.
#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({ "message": "Hello from RustyAPI" }))
}

/// Список задач.
#[get("")]
async fn list_todos(db: Db, Query(filter): Query<TodoFilter>) -> Result<Json<Vec<Todo>>> {
    let todos = Todo::select()
        .eq_opt("done", filter.done)
        .order_by_desc("id")
        .limit(filter.limit)
        .offset(filter.offset)
        .all(&db)
        .await?;
    Ok(Json(todos))
}

/// Одна задача по id.
#[get("/{todo_id}")]
async fn read_todo(db: Db, Path(todo_id): Path<i64>) -> Result<Json<Todo>> {
    Ok(Json(Todo::get_or_404(&db, todo_id).await?))
}

/// Создать задачу.
#[post("", status_code = 201)]
async fn create_todo(db: Db, Json(body): Json<TodoCreate>) -> Result<Json<Todo>> {
    let todo = Todo::insert().set("title", body.title).execute(&db).await?;
    Ok(Json(todo))
}

/// Изменить задачу (PATCH: только переданные поля).
#[patch("/{todo_id}")]
async fn update_todo(
    db: Db,
    Path(todo_id): Path<i64>,
    Json(body): Json<TodoUpdate>,
) -> Result<Json<Todo>> {
    let todo = Todo::update(todo_id)
        .set_opt("title", body.title)
        .set_opt("done", body.done)
        .execute(&db)
        .await?;
    Ok(Json(todo))
}

/// Удалить задачу.
#[delete("/{todo_id}", status_code = status::HTTP_204_NO_CONTENT)]
async fn delete_todo(db: Db, Path(todo_id): Path<i64>) -> Result<()> {
    Todo::delete(&db, todo_id).await?;
    Ok(())
}

fn routes() -> App {
    let todos = APIRouter::new()
        .prefix("/todos")
        .tags(["todos"])
        .route(list_todos)
        .route(read_todo)
        .route(create_todo)
        .route(update_todo)
        .route(delete_todo);

    App::new()
        .title("Todo API")
        .version("0.1.0")
        .migrations(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .route(root)
        .include_router(todos)
}

#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    let _ = rustyapi::dotenvy::dotenv();
    routes().database_from_env().run().await
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use rustyapi::axum::body::Body;
    use rustyapi::axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    /// Пул на отдельной схеме тестовой базы с применёнными миграциями; `None` без `DATABASE_URL`.
    async fn test_pool() -> Option<rustyapi::db::Pool> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL не задан — тест пропущен");
            return None;
        };
        let admin = rustyapi::db::pool_from_url(&url).unwrap();
        rustyapi::sqlx::raw_sql(
            "DROP SCHEMA IF EXISTS todo_api_test CASCADE; CREATE SCHEMA todo_api_test;",
        )
        .execute(&admin)
        .await
        .unwrap();
        let sep = if url.contains('?') { '&' } else { '?' };
        let pool = rustyapi::db::pool_from_url(&format!(
            "{url}{sep}options=-c%20search_path%3Dtodo_api_test"
        ))
        .unwrap();
        routes()
            .database(pool.clone())
            .run_migrations()
            .await
            .unwrap();
        Some(pool)
    }

    async fn call(
        pool: &rustyapi::db::Pool,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                request = request.header("content-type", "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = routes()
            .database(pool.clone())
            .into_router()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            rustyapi::serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn crud_flow() {
        let Some(pool) = test_pool().await else {
            return;
        };
        assert_eq!(
            call(&pool, "GET", "/", None).await,
            (200, json!({"message": "Hello from RustyAPI"}))
        );

        let (status, todo) = call(
            &pool,
            "POST",
            "/todos",
            Some(json!({"title": "Купить хлеб"})),
        )
        .await;
        assert_eq!(status, 201);
        let id = todo["id"].as_i64().unwrap();
        assert_eq!(todo["done"], json!(false));

        let (status, todo) = call(
            &pool,
            "PATCH",
            &format!("/todos/{id}"),
            Some(json!({"done": true})),
        )
        .await;
        assert_eq!(
            (status, &todo["done"], &todo["title"]),
            (200, &json!(true), &json!("Купить хлеб"))
        );

        let (status, list) = call(&pool, "GET", "/todos?done=true", None).await;
        assert_eq!((status, list.as_array().unwrap().len()), (200, 1));
        let (_, list) = call(&pool, "GET", "/todos?done=false", None).await;
        assert_eq!(list, json!([]));

        assert_eq!(
            call(&pool, "DELETE", &format!("/todos/{id}"), None).await,
            (204, Value::Null)
        );
        assert_eq!(
            call(&pool, "GET", &format!("/todos/{id}"), None).await,
            (404, json!({"detail": "Todo not found"}))
        );
        assert_eq!(call(&pool, "GET", "/todos/", None).await.0, 307);
        assert_eq!(
            call(&pool, "GET", "/nope", None).await,
            (404, json!({"detail": "Not Found"}))
        );
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn validation_errors() {
        // Ошибки 422 возникают до обращения к базе: подойдёт ленивый пул без сервера.
        let pool = rustyapi::db::pool_from_url("postgres://nobody@127.0.0.1:1/none").unwrap();
        let (status, body) = call(&pool, "POST", "/todos", Some(json!({"title": ""}))).await;
        assert_eq!(status, 422);
        assert_eq!(body["detail"][0]["type"], "string_too_short");
        assert_eq!(body["detail"][0]["loc"], json!(["body", "title"]));

        let (status, body) = call(&pool, "GET", "/todos?limit=0", None).await;
        assert_eq!(status, 422);
        assert_eq!(body["detail"][0]["loc"], json!(["query", "limit"]));

        let (status, body) = call(&pool, "GET", "/todos/abc", None).await;
        assert_eq!(status, 422);
        assert_eq!(body["detail"][0]["type"], "int_parsing");
    }

    #[test]
    fn openapi_has_models() {
        let doc = routes().openapi();
        assert!(doc["components"]["schemas"]["Todo"].is_object());
        assert_eq!(
            doc["paths"]["/todos/{todo_id}"]["patch"]["operationId"],
            "update_todo_todos__todo_id__patch"
        );
    }
}
