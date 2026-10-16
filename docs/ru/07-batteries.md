# Батарейки

## Middleware

```rust
App::new()
    .cors_origins(["https://app.example.com"])   // или .cors_allow_all() для разработки
    .gzip()                                     // ответы > 500 байт, если клиент принимает gzip
    .body_limit(10 * 1024 * 1024)               // 10 МБ; больше — 413 (по умолчанию 2 МБ)
    .timeout(Duration::from_secs(30))           // не успели — 504 {"detail": "Gateway Timeout"}
    .rate_limit(RateLimit::per_minute(60).burst(10))  // по IP; 429 + Retry-After
```

`RateLimit` хранит счётчики в памяти процесса. За reverse proxy добавьте `.trust_forwarded_for()`,
иначе все запросы придут с IP прокси.

## Статика

```rust
App::new().mount("/static", "static")   // static/css/app.css → /static/css/app.css
```

Нет файла — `404 {"detail": "Not Found"}`, как у `StaticFiles` в FastAPI.

## Настройки

Аналог `pydantic-settings`: поле `database_url` читается из `DATABASE_URL`, списки через запятую,
правила `#[field]` проверяются при старте.

```rust
#[schema]
struct Settings {
    database_url: String,
    #[field(default = 8000, ge = 1)]
    port: u16,
    #[field(default = Vec::new())]
    cors_origins: Vec<String>,
}

let settings: Settings = rustyapi::settings()?;   // .env + окружение
```

## TestClient

```rust
let client = TestClient::new(app());
let response = client.post("/items").json(&body).bearer(&token).send().await;  // или просто .await
assert_eq!(response.status(), 201);
let item: Item = response.json();
```

Есть `.get/.put/.patch/.delete`, `.query(&params)`, `.form(&data)`, `.header(k, v)`,
`.multipart(MultipartBody::new().file("file", "a.txt", "text/plain", bytes))`.

## Фоновые задачи

```rust
#[post("/signup")]
async fn signup(tasks: BackgroundTasks, Json(user): Json<UserCreate>) -> Json<Value> {
    tasks.add(async move { send_welcome_email(&user.email).await });  // Result<_, E> → ошибка в лог
    Json(json!({"ok": true}))
}
```

Задачи выполняются по очереди после ответа. Если обработчик вернул ошибку (статус ≥ 400), задачи не
запускаются, как в FastAPI.

## WebSocket

```rust
#[websocket("/ws/{room}")]
async fn chat(mut socket: WebSocket, Path(room): Path<String>, auth: Auth) {
    while let Some(Ok(Message::Text(text))) = socket.recv().await {
        socket.send(Message::Text(format!("{room}: {text}").into())).await.ok();
    }
}
```

Экстракторы после сокета (`Path`, `Query`, `Auth`, `Depends`) выполняются до установки соединения.

## Загрузка файлов

```rust
#[post("/upload")]
async fn upload(file: UploadFile) -> Result<Json<Value>> {   // первый файл формы
    file.save(format!("uploads/{}", uuid::Uuid::new_v4())).await?;
    Ok(Json(json!({"filename": file.filename(), "size": file.size()})))
}

#[post("/profile")]
async fn profile(form: MultipartForm) -> Result<Json<Value>> {
    let avatar = form.file("avatar")?;          // нет — 422 как в FastAPI
    let name = form.require_text("name")?;
    Ok(Json(json!({"name": name})))
}
```

## Кэш

```rust
App::new().cache(Cache::memory())                         // в памяти процесса
App::new().cache(Cache::redis("redis://127.0.0.1/").await?)  // фича `redis`

#[get("/stats")]
async fn stats(cache: Cache) -> Result<Json<Stats>> {
    Ok(Json(cache.get_or_set("stats", Duration::from_secs(60), || load_stats()).await?))
}
```

Также `get::<T>(key)`, `set(key, &value, ttl)`, `delete(key)`, `with_prefix("app:")`.

## TypeScript-клиент

```sh
tez generate client --ts --out web/src/api.ts
```

Интерфейсы для всех схем и класс `Client` с методом на каждый эндпоинт (`listTodos`, `createTodo`...);
ошибки — `ApiError` со `status` и `detail`. Работает на `fetch`.

```ts
const api = new Client({ baseUrl: "http://127.0.0.1:8000" });
api.setToken((await api.login({ username, password })).access_token);
const todos = await api.listTodos({ done: false });
```
