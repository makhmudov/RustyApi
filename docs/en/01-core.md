# Core: app, routes, errors

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

* `App::run()` listens on `HOST:PORT` (default `127.0.0.1:8000`) and shuts down gracefully on Ctrl+C / SIGTERM.
  Logs follow the uvicorn format; `RUST_LOG` sets the level, `NO_COLOR` disables colors.
* Route macros: `get`, `post`, `put`, `patch`, `delete`, `head`, `options` with `status_code`, `tags`,
  `summary`, `description`, `deprecated`. The doc comment becomes the description. `{name:path}` matches the rest of the path.
* `APIRouter::new().prefix("/items").tags(["items"]).route(...)`, then `app.include_router(router)`.
  A path that differs from a route only by a trailing `/` gets a `307` redirect, like FastAPI's `redirect_slashes`.
* `HTTPException::new(404, "Item not found")` responds with `{"detail": ...}`; `.with_header(...)` adds headers.
  `rustyapi::Result<T>` accepts any error through `?`: an `HTTPException` is returned as is, anything else becomes
  `500 {"detail": "Internal Server Error"}` and is logged.
* Unknown path: `404 {"detail": "Not Found"}`. Wrong method: `405 {"detail": "Method Not Allowed"}` with `Allow`.
  A panic in a handler: `500 {"detail": "Internal Server Error"}`.

See the Russian page [docs/ru/01-core.md](../ru/01-core.md) for the FastAPI ↔ RustyAPI table.
