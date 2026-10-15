# Batteries

## Middleware

```rust
App::new()
    .cors_origins(["https://app.example.com"])   // or .cors_allow_all() for development
    .gzip()                                     // responses > 500 bytes when the client accepts gzip
    .body_limit(10 * 1024 * 1024)               // 10 MB; larger bodies get 413 (default 2 MB)
    .timeout(Duration::from_secs(30))           // too slow: 504 {"detail": "Gateway Timeout"}
    .rate_limit(RateLimit::per_minute(60).burst(10))  // per IP; 429 + Retry-After
```

`RateLimit` keeps counters in process memory. Behind a reverse proxy add `.trust_forwarded_for()`,
otherwise every request comes from the proxy's IP.

## Static files

```rust
App::new().mount("/static", "static")   // static/css/app.css → /static/css/app.css
```

A missing file returns `404 {"detail": "Not Found"}`, like FastAPI's `StaticFiles`.

## Settings

Like `pydantic-settings`: the field `database_url` reads `DATABASE_URL`, lists are comma-separated,
and `#[field]` rules are checked at startup.

```rust
#[schema]
struct Settings {
    database_url: String,
    #[field(default = 8000, ge = 1)]
    port: u16,
    #[field(default = Vec::new())]
    cors_origins: Vec<String>,
}

let settings: Settings = rustyapi::settings()?;   // .env + environment
```

## TestClient

```rust
let client = TestClient::new(app());
let response = client.post("/items").json(&body).bearer(&token).send().await;  // or just .await
assert_eq!(response.status(), 201);
let item: Item = response.json();
```

Also `.get/.put/.patch/.delete`, `.query(&params)`, `.form(&data)`, `.header(k, v)`,
`.multipart(MultipartBody::new().file("file", "a.txt", "text/plain", bytes))`.

## Background tasks

```rust
#[post("/signup")]
async fn signup(tasks: BackgroundTasks, Json(user): Json<UserCreate>) -> Json<Value> {
    tasks.add(async move { send_welcome_email(&user.email).await });  // Result<_, E> errors are logged
    Json(json!({"ok": true}))
}
```

Tasks run in order after the response. If the handler returns an error (status ≥ 400) they don't
run, as in FastAPI.

## WebSocket

```rust
#[websocket("/ws/{room}")]
async fn chat(mut socket: WebSocket, Path(room): Path<String>, auth: Auth) {
    while let Some(Ok(Message::Text(text))) = socket.recv().await {
        socket.send(Message::Text(format!("{room}: {text}").into())).await.ok();
    }
}
```

Extractors after the socket (`Path`, `Query`, `Auth`, `Depends`) run before the upgrade.

## File uploads

```rust
#[post("/upload")]
async fn upload(file: UploadFile) -> Result<Json<Value>> {   // first file in the form
    file.save(format!("uploads/{}", uuid::Uuid::new_v4())).await?;
    Ok(Json(json!({"filename": file.filename(), "size": file.size()})))
}

#[post("/profile")]
async fn profile(form: MultipartForm) -> Result<Json<Value>> {
    let avatar = form.file("avatar")?;          // missing: 422 like FastAPI
    let name = form.require_text("name")?;
    Ok(Json(json!({"name": name})))
}
```

## Cache

```rust
App::new().cache(Cache::memory())                            // in process memory
App::new().cache(Cache::redis("redis://127.0.0.1/").await?)  // `redis` feature

#[get("/stats")]
async fn stats(cache: Cache) -> Result<Json<Stats>> {
    Ok(Json(cache.get_or_set("stats", Duration::from_secs(60), || load_stats()).await?))
}
```

Also `get::<T>(key)`, `set(key, &value, ttl)`, `delete(key)`, `with_prefix("app:")`.

## TypeScript client

```sh
tez generate client --ts --out web/src/api.ts
```

Interfaces for every schema and a `Client` class with one method per endpoint (`listTodos`,
`createTodo`...); errors are `ApiError` with `status` and `detail`. Built on `fetch`.

```ts
const api = new Client({ baseUrl: "http://127.0.0.1:8000" });
api.setToken((await api.login({ username, password })).access_token);
const todos = await api.listTodos({ done: false });
```
