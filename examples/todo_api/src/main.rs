//! Эталонный пример RustyAPI (JWT + PostgreSQL + CRUD). Наполняется по мере этапов ROADMAP.md.
//!
//! Этап 1: роуты, `APIRouter`, `HTTPException`, `status_code`. Пока данные хранятся в памяти;
//! на этапе 4 их заменит PostgreSQL.
//!
//! ```text
//! cargo run -p todo_api
//! curl http://127.0.0.1:8000/
//! curl -X POST http://127.0.0.1:8000/todos -H 'content-type: application/json' -d '{"title":"Купить хлеб"}'
//! ```

use std::sync::{Mutex, MutexGuard};

use rustyapi::prelude::*;
use rustyapi::serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(crate = "rustyapi::serde")]
struct Todo {
    id: i64,
    title: String,
    done: bool,
}

#[derive(Debug, Deserialize)]
#[serde(crate = "rustyapi::serde")]
struct TodoCreate {
    title: String,
}

static TODOS: Mutex<Vec<Todo>> = Mutex::new(Vec::new());

fn todos() -> MutexGuard<'static, Vec<Todo>> {
    TODOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn not_found() -> HTTPException {
    HTTPException::new(status::HTTP_404_NOT_FOUND, "Todo not found")
}

/// Приветствие.
#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({ "message": "Hello from RustyAPI" }))
}

/// Список задач.
#[get("")]
async fn list_todos() -> Json<Vec<Todo>> {
    Json(todos().clone())
}

/// Одна задача по id.
#[get("/{todo_id}")]
async fn read_todo(Path(todo_id): Path<i64>) -> Result<Json<Todo>> {
    let todo = todos().iter().find(|t| t.id == todo_id).cloned();
    Ok(Json(todo.ok_or_else(not_found)?))
}

/// Создать задачу.
#[post("", status_code = 201)]
async fn create_todo(Json(body): Json<TodoCreate>) -> Json<Todo> {
    let mut todos = todos();
    let id = todos.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    let todo = Todo {
        id,
        title: body.title,
        done: false,
    };
    todos.push(todo.clone());
    Json(todo)
}

/// Отметить задачу выполненной.
#[patch("/{todo_id}/done", summary = "Mark Todo Done")]
async fn mark_done(Path(todo_id): Path<i64>) -> Result<Json<Todo>> {
    let mut todos = todos();
    let todo = todos
        .iter_mut()
        .find(|t| t.id == todo_id)
        .ok_or_else(not_found)?;
    todo.done = true;
    Ok(Json(todo.clone()))
}

/// Удалить задачу.
#[delete("/{todo_id}", status_code = status::HTTP_204_NO_CONTENT)]
async fn delete_todo(Path(todo_id): Path<i64>) -> Result<()> {
    let mut todos = todos();
    let index = todos
        .iter()
        .position(|t| t.id == todo_id)
        .ok_or_else(not_found)?;
    todos.remove(index);
    Ok(())
}

fn app() -> App {
    let todos = APIRouter::new()
        .prefix("/todos")
        .tags(["todos"])
        .route(list_todos)
        .route(read_todo)
        .route(create_todo)
        .route(mark_done)
        .route(delete_todo);

    App::new()
        .title("Todo API")
        .version("0.1.0")
        .route(root)
        .include_router(todos)
}

#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    app().run().await
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use rustyapi::axum::body::Body;
    use rustyapi::axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    async fn call(method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
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
        assert_eq!(
            call("GET", "/", None).await,
            (200, json!({"message": "Hello from RustyAPI"}))
        );

        let (status, todo) = call("POST", "/todos", Some(json!({"title": "Купить хлеб"}))).await;
        assert_eq!(status, 201);
        let id = todo["id"].as_i64().unwrap();

        let (status, todo) = call("PATCH", &format!("/todos/{id}/done"), None).await;
        assert_eq!((status, todo["done"].clone()), (200, json!(true)));

        assert_eq!(
            call("DELETE", &format!("/todos/{id}"), None).await,
            (204, Value::Null)
        );
        assert_eq!(
            call("GET", &format!("/todos/{id}"), None).await,
            (404, json!({"detail": "Todo not found"}))
        );
        assert_eq!(call("GET", "/todos/", None).await.0, 307);
        assert_eq!(
            call("GET", "/nope", None).await,
            (404, json!({"detail": "Not Found"}))
        );
    }
}
