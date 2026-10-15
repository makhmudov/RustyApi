# Ядро: приложение, роуты, ошибки

## Hello world

```rust
use rustyapi::prelude::*;

#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({"message": "Hello"}))
}

#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    App::new().title("My API").route(root).run().await
}
```

В `Cargo.toml` нужна только одна зависимость — `rustyapi`.

## Запуск

`App::run()` слушает `HOST:PORT` из переменных окружения (по умолчанию `127.0.0.1:8000`, как uvicorn)
и корректно останавливается по Ctrl+C или SIGTERM, дожидаясь текущих запросов.

```text
INFO:     Started server process [4242]
INFO:     Waiting for application startup.
INFO:     Application startup complete.
INFO:     RustyAPI running on http://127.0.0.1:8000 (Press CTRL+C to quit)
INFO:     127.0.0.1:51234 - "GET / HTTP/1.1" 200 OK
```

Уровень логов задаётся через `RUST_LOG` (по умолчанию `info`), `NO_COLOR` отключает цвета.
Для тестов и встраивания: `app.serve(listener)`, `app.serve_with_shutdown(listener, signal)`,
`app.into_router()` (обычный `axum::Router`).

## Маршруты

| FastAPI | RustyAPI |
|---|---|
| `@app.get("/items/{item_id}")` | `#[get("/items/{item_id}")]` |
| `@app.post("/items", status_code=201)` | `#[post("/items", status_code = 201)]` |
| `status_code=status.HTTP_204_NO_CONTENT` | `status_code = status::HTTP_204_NO_CONTENT` |
| `tags=["items"]`, `summary="..."`, `deprecated=True` | `tags = ["items"]`, `summary = "..."`, `deprecated` |
| `"/files/{file_path:path}"` | `"/files/{file_path:path}"` |

Есть `get`, `post`, `put`, `patch`, `delete`, `head`, `options`. Doc-комментарий функции становится
описанием (`description`) эндпоинта. `status_code` применяется к успешному ответу; `HTTPException`
и явно возвращённый статус его не трогают.

Макрос превращает функцию в unit-struct с тем же именем; регистрируется он так: `app.route(read_item)`.

## APIRouter

```rust
let items = APIRouter::new()
    .prefix("/items")
    .tags(["items"])
    .route(list_items)   // #[get("")]  → /items
    .route(read_item);   // #[get("/{item_id}")] → /items/{item_id}

App::new().include_router(items)
```

Как в FastAPI, префикс начинается с `/` и не заканчивается на `/`; роутеры можно вкладывать друг в друга.
Если путь отличается от маршрута только завершающим `/`, ответ — `307` на правильный адрес (`redirect_slashes`).

## Ошибки

```rust
#[get("/items/{item_id}")]
async fn read_item(Path(item_id): Path<u32>) -> Result<Json<Item>> {
    let item = find(item_id).ok_or_else(|| HTTPException::new(404, "Item not found"))?;
    Ok(Json(item))
}
```

* `HTTPException::new(status, detail)` → `{"detail": ...}`; `detail` может быть любым JSON.
  `.with_header("WWW-Authenticate", "Bearer")` добавляет заголовок.
* `rustyapi::Result<T>` — `Result<T, rustyapi::Error>`. Через `?` в него превращается любая ошибка;
  `HTTPException` отдаётся как есть, остальное — `500 {"detail": "Internal Server Error"}` с записью в лог.
* Неизвестный путь → `404 {"detail": "Not Found"}`, неподходящий метод → `405 {"detail": "Method Not Allowed"}`
  с заголовком `Allow`, паника в обработчике → `500 {"detail": "Internal Server Error"}`.
