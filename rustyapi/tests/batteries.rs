//! Этап 7: CORS, gzip, лимиты, таймаут, статика, настройки, TestClient, фоновые задачи,
//! WebSocket, загрузка файлов, кэш, rate limiting.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rustyapi::prelude::*;
use rustyapi::ratelimit::RateLimit;
use rustyapi::testing::{MultipartBody, TestClient};
use rustyapi::websocket::{Message, WebSocket};

#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({"ok": true}))
}

#[get("/big")]
async fn big() -> String {
    "RustyAPI ".repeat(200)
}

#[post("/echo")]
async fn echo(Json(body): Json<Value>) -> Json<Value> {
    Json(body)
}

#[get("/slow")]
async fn slow() -> Json<Value> {
    rustyapi::tokio::time::sleep(Duration::from_millis(300)).await;
    Json(json!({}))
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn cors() {
    let client = TestClient::new(App::new().cors_allow_all().route(root));
    let preflight = client
        .options("/")
        .header("origin", "https://site.example")
        .header("access-control-request-method", "POST")
        .send()
        .await;
    assert_eq!(preflight.status(), 200);
    assert_eq!(preflight.header("access-control-allow-origin"), Some("*"));
    let response = client.get("/").header("origin", "https://a.example").await;
    assert_eq!(response.header("access-control-allow-origin"), Some("*"));

    let client = TestClient::new(
        App::new()
            .cors_origins(["https://good.example"])
            .route(root),
    );
    let good = client
        .get("/")
        .header("origin", "https://good.example")
        .await;
    assert_eq!(
        good.header("access-control-allow-origin"),
        Some("https://good.example")
    );
    assert_eq!(
        good.header("access-control-allow-credentials"),
        Some("true")
    );
    let bad = client
        .get("/")
        .header("origin", "https://evil.example")
        .await;
    assert_eq!(bad.header("access-control-allow-origin"), None);
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn gzip_limits_timeout() {
    let client = TestClient::new(
        App::new()
            .gzip()
            .body_limit(64)
            .timeout(Duration::from_millis(50))
            .route(root)
            .route(big)
            .route(echo)
            .route(slow),
    );
    let compressed = client.get("/big").header("accept-encoding", "gzip").await;
    assert_eq!(compressed.header("content-encoding"), Some("gzip"));
    assert!(compressed.bytes().len() < 1800);
    let small = client.get("/").header("accept-encoding", "gzip").await;
    assert_eq!(
        small.header("content-encoding"),
        None,
        "меньше 500 байт не сжимается"
    );

    let ok = client.post("/echo").json(&json!({"a": 1})).await;
    assert_eq!(ok.status(), 200);
    let too_big = client
        .post("/echo")
        .json(&json!({"a": "x".repeat(100)}))
        .await;
    assert_eq!(too_big.status(), 413);
    assert_eq!(
        too_big.json::<Value>(),
        json!({"detail": "Payload Too Large"})
    );

    let timeout = client.get("/slow").await;
    assert_eq!(
        (timeout.status_code(), timeout.json::<Value>()),
        (504, json!({"detail": "Gateway Timeout"}))
    );
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn static_files() {
    let dir = std::env::temp_dir().join(format!("rustyapi_static_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("css")).unwrap();
    std::fs::write(dir.join("css/app.css"), "body { color: red }").unwrap();
    let client = TestClient::new(App::new().mount("/static", &dir).route(root));
    let css = client.get("/static/css/app.css").await;
    assert_eq!(css.status(), 200);
    assert_eq!(css.text(), "body { color: red }");
    assert_eq!(css.header("content-type"), Some("text/css"));
    let missing = client.get("/static/nope.js").await;
    assert_eq!(
        (missing.status_code(), missing.json::<Value>()),
        (404, json!({"detail": "Not Found"}))
    );
    assert_eq!(client.get("/").await.status(), 200);
    std::fs::remove_dir_all(dir).unwrap();
}

#[schema]
struct Settings {
    database_url: String,
    #[field(default = 8000, ge = 1)]
    port: u16,
    #[field(default = Vec::new())]
    cors_origins: Vec<String>,
    debug: Option<bool>,
}

#[test]
fn settings() {
    let vars = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    };
    let s: Settings = rustyapi::settings::settings_from(vars(&[
        ("DATABASE_URL", "postgres://x"),
        ("CORS_ORIGINS", "https://a.com,https://b.com"),
        ("DEBUG", "true"),
        ("UNRELATED", "1"),
    ]))
    .unwrap();
    assert_eq!(
        (
            s.database_url.as_str(),
            s.port,
            s.cors_origins.len(),
            s.debug
        ),
        ("postgres://x", 8000, 2, Some(true))
    );
    let err = rustyapi::settings::settings_from::<Settings, _>(vars(&[])).unwrap_err();
    assert!(err.to_string().contains("DATABASE_URL"), "{err}");
    let err = rustyapi::settings::settings_from::<Settings, _>(vars(&[
        ("DATABASE_URL", "x"),
        ("PORT", "0"),
    ]))
    .unwrap_err();
    assert!(err.to_string().contains("PORT"), "{err}");
    let err = rustyapi::settings::settings_from::<Settings, _>(vars(&[
        ("DATABASE_URL", "x"),
        ("PORT", "abc"),
    ]))
    .unwrap_err();
    assert!(err.to_string().contains("settings"), "{err}");
}

#[schema]
struct Search {
    q: String,
    #[field(default = 10)]
    limit: i64,
}

#[get("/search")]
async fn search(Query(s): Query<Search>, auth: Option<Auth>) -> Json<Value> {
    Json(json!({"q": s.q, "limit": s.limit, "user": auth.map(|a| a.sub().to_owned())}))
}

#[schema]
struct Login {
    username: String,
}

#[post("/login")]
async fn login(Form(f): Form<Login>) -> Json<Value> {
    Json(json!({"username": f.username}))
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn test_client() {
    rustyapi::auth::set_secret_key("batteries-secret");
    let client = TestClient::new(App::new().route(search).route(login));
    let token = rustyapi::auth::create_access_token("azim").unwrap();
    let response = client
        .get("/search")
        .query(&json!({"q": "rust"}))
        .bearer(&token)
        .send()
        .await;
    assert_eq!(
        response.json::<Value>(),
        json!({"q": "rust", "limit": 10, "user": "azim"})
    );
    let response = client
        .post("/login")
        .form(&json!({"username": "a b"}))
        .await;
    assert_eq!(response.json::<Value>(), json!({"username": "a b"}));
    assert_eq!(client.get("/search").await.status_code(), 422);
}

static DONE: AtomicUsize = AtomicUsize::new(0);

#[post("/signup")]
async fn signup(tasks: BackgroundTasks, Json(body): Json<Value>) -> Result<Json<Value>> {
    tasks.add(async {
        DONE.fetch_add(1, Ordering::SeqCst);
    });
    tasks.add(async {
        DONE.fetch_add(10, Ordering::SeqCst);
        Err::<(), _>("ошибка задачи попадает в лог")
    });
    if body["fail"] == true {
        return Err(HTTPException::new(400, "bad").into());
    }
    Ok(Json(json!({"queued": tasks.len()})))
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn background_tasks() {
    let client = TestClient::new(App::new().route(signup));
    let response = client.post("/signup").json(&json!({})).await;
    assert_eq!(response.json::<Value>(), json!({"queued": 2}));
    for _ in 0..100 {
        if DONE.load(Ordering::SeqCst) == 11 {
            break;
        }
        rustyapi::tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(DONE.load(Ordering::SeqCst), 11);
    // При ошибке задачи не запускаются (как в FastAPI).
    assert_eq!(
        client
            .post("/signup")
            .json(&json!({"fail": true}))
            .await
            .status(),
        400
    );
    rustyapi::tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(DONE.load(Ordering::SeqCst), 11);
}

#[post("/upload")]
async fn upload(file: UploadFile) -> Json<Value> {
    Json(json!({
        "field": file.field_name(),
        "filename": file.filename(),
        "content_type": file.content_type(),
        "size": file.size(),
        "text": file.text().ok(),
    }))
}

#[post("/profile")]
async fn profile(form: MultipartForm) -> Result<Json<Value>> {
    let avatar = form.file("avatar")?;
    let name = form.require_text("name")?;
    Ok(Json(
        json!({"name": name, "avatar": avatar.filename(), "docs": form.files("doc").len()}),
    ))
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn uploads() {
    let client = TestClient::new(App::new().route(upload).route(profile));
    let response = client
        .post("/upload")
        .multipart(MultipartBody::new().file("file", "hello.txt", "text/plain", "привет"))
        .await;
    assert_eq!(
        response.json::<Value>(),
        json!({"field": "file", "filename": "hello.txt", "content_type": "text/plain", "size": 12, "text": "привет"})
    );
    let missing = client.post("/upload").json(&json!({})).await;
    assert_eq!(missing.status(), 422);
    assert_eq!(
        missing.json::<Value>(),
        json!({"detail": [{"type": "missing", "loc": ["body", "file"], "msg": "Field required", "input": null}]})
    );

    let form = MultipartBody::new()
        .text("name", "Азим")
        .file(
            "avatar",
            "me.png",
            "image/png",
            vec![0x89, b'P', b'N', b'G'],
        )
        .file("doc", "a.pdf", "application/pdf", "a")
        .file("doc", "b.pdf", "application/pdf", "b");
    let response = client.post("/profile").multipart(form).await;
    assert_eq!(
        response.json::<Value>(),
        json!({"name": "Азим", "avatar": "me.png", "docs": 2})
    );
    let response = client
        .post("/profile")
        .multipart(MultipartBody::new().text("name", "x"))
        .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        response.json::<Value>()["detail"][0]["loc"],
        json!(["body", "avatar"])
    );

    let doc = App::new().route(upload).openapi();
    assert_eq!(
        doc["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data"]["schema"]
            ["properties"]["file"],
        json!({"type": "string", "format": "binary", "title": "File"})
    );
}

static COMPUTED: AtomicUsize = AtomicUsize::new(0);

#[get("/stats")]
async fn stats(cache: Cache) -> Result<Json<Value>> {
    let value = cache
        .get_or_set("stats", Duration::from_secs(60), || async {
            COMPUTED.fetch_add(1, Ordering::SeqCst);
            Ok::<_, rustyapi::Error>(json!({"users": 42}))
        })
        .await?;
    Ok(Json(value))
}

async fn exercise_cache(cache: Cache) {
    cache.delete("k").await.unwrap();
    assert_eq!(cache.get::<String>("k").await.unwrap(), None);
    cache.set("k", "v", None).await.unwrap();
    assert_eq!(
        cache.get::<String>("k").await.unwrap().as_deref(),
        Some("v")
    );
    // Другой тип — None, а не ошибка.
    assert_eq!(cache.get::<i64>("k").await.unwrap(), None);
    cache
        .set("short", &json!([1, 2]), Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(
        cache.get::<Vec<i32>>("short").await.unwrap(),
        Some(vec![1, 2])
    );
    rustyapi::tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(cache.get::<Vec<i32>>("short").await.unwrap(), None);
    cache.delete("k").await.unwrap();
    assert_eq!(cache.get::<String>("k").await.unwrap(), None);
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn cache_memory() {
    exercise_cache(Cache::memory().with_prefix("test:")).await;
    let client = TestClient::new(App::new().cache(Cache::memory()).route(stats));
    for _ in 0..3 {
        assert_eq!(
            client.get("/stats").await.json::<Value>(),
            json!({"users": 42})
        );
    }
    assert_eq!(COMPUTED.load(Ordering::SeqCst), 1);
    // Без App::cache — 500 с понятным логом.
    assert_eq!(
        TestClient::new(App::new().route(stats))
            .get("/stats")
            .await
            .status(),
        500
    );
}

#[cfg(feature = "redis")]
#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn cache_redis() {
    let Ok(url) = std::env::var("REDIS_URL") else {
        eprintln!("REDIS_URL не задан, тест пропущен");
        return;
    };
    let cache = Cache::redis(&url)
        .await
        .unwrap()
        .with_prefix(format!("rustyapi-test-{}:", std::process::id()));
    exercise_cache(cache).await;
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn rate_limit() {
    let client = TestClient::new(App::new().rate_limit(RateLimit::per_minute(2)).route(root));
    let first = client.get("/").await;
    assert_eq!(first.header("x-ratelimit-limit"), Some("2"));
    assert_eq!(first.header("x-ratelimit-remaining"), Some("1"));
    assert_eq!(client.get("/").await.status(), 200);
    let limited = client.get("/").await;
    assert_eq!(
        (limited.status_code(), limited.json::<Value>()),
        (429, json!({"detail": "Too Many Requests"}))
    );
    assert_eq!(limited.header("retry-after"), Some("30"));
}

#[websocket("/ws/{room}")]
async fn chat(mut socket: WebSocket, Path(room): Path<String>) {
    while let Some(Ok(Message::Text(text))) = socket.recv().await {
        let reply = format!("{room}: {text}");
        if socket.send(Message::Text(reply.into())).await.is_err() {
            break;
        }
    }
}

/// Минимальный WebSocket-клиент: рукопожатие и короткие текстовые кадры.
mod ws_client {
    use rustyapi::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use rustyapi::tokio::net::TcpStream;

    pub async fn connect(addr: std::net::SocketAddr, path: &str) -> TcpStream {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
        stream
    }

    pub async fn send_text(stream: &mut TcpStream, text: &str) {
        let payload = text.as_bytes();
        assert!(payload.len() < 126);
        let mask = [1u8, 2, 3, 4];
        let mut frame = vec![0x81, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        stream.write_all(&frame).await.unwrap();
    }

    pub async fn recv_text(stream: &mut TcpStream) -> String {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header).await.unwrap();
        assert_eq!(header[0], 0x81, "текстовый кадр");
        let len = usize::from(header[1] & 0x7f);
        assert!(len < 126);
        let mut payload = vec![0; len];
        stream.read_exact(&mut payload).await.unwrap();
        String::from_utf8(payload).unwrap()
    }
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn websocket() {
    let app = App::new().route(chat).route(root);
    // В OpenAPI WebSocket не попадает (как в FastAPI).
    assert!(app.openapi()["paths"].get("/ws/{room}").is_none());
    let listener = rustyapi::tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = rustyapi::tokio::sync::oneshot::channel::<()>();
    let server = rustyapi::tokio::spawn(app.serve_with_shutdown(listener, async {
        let _ = stopped.await;
    }));
    let mut socket = ws_client::connect(addr, "/ws/general").await;
    ws_client::send_text(&mut socket, "салам").await;
    assert_eq!(ws_client::recv_text(&mut socket).await, "general: салам");
    ws_client::send_text(&mut socket, "ещё").await;
    assert_eq!(ws_client::recv_text(&mut socket).await, "general: ещё");
    drop(socket);
    let _ = stop.send(());
    server.await.unwrap().unwrap();
}
