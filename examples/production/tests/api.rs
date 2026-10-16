//! Интеграционные тесты. Тесты с базой пропускаются без `DATABASE_URL`.

use std::time::Duration;

use rustyapi::App;
use rustyapi::auth::Token;
use rustyapi::cache::Cache;
use rustyapi::prelude::{Value, json};
use rustyapi::ratelimit::RateLimit;
use rustyapi::testing::TestClient;
use shortener::Settings;
use shortener::links::Link;

/// Приложение на отдельной схеме тестовой базы с применёнными миграциями.
async fn test_app(schema: &str) -> Option<App> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL не задан — тест пропущен");
        return None;
    };
    let admin = rustyapi::db::pool_from_url(&url).unwrap();
    rustyapi::sqlx::raw_sql(&format!(
        "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};"
    ))
    .execute(&admin)
    .await
    .unwrap();
    let sep = if url.contains('?') { '&' } else { '?' };
    let pool =
        rustyapi::db::pool_from_url(&format!("{url}{sep}options=-c%20search_path%3D{schema}"))
            .unwrap();
    let app = shortener::routes()
        .database(pool)
        .migrations(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .cache(Cache::memory());
    app.run_migrations().await.unwrap();
    rustyapi::auth::set_secret_key("shortener-test-secret-0123456789abcdef");
    Some(app)
}

async fn sign_up(client: &TestClient, username: &str) -> String {
    let user = client
        .post("/users")
        .json(&json!({"username": username, "password": "supersecret"}))
        .await;
    assert_eq!(user.status(), 201, "{}", user.text());
    client
        .post("/token")
        .form(&json!({"username": username, "password": "supersecret"}))
        .await
        .json::<Token>()
        .access_token
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn shorten_follow_and_count_clicks() {
    let Some(app) = test_app("shortener_flow_test").await else {
        return;
    };
    let api = TestClient::new(app);
    let token = sign_up(&api, "azim").await;

    let created = api
        .post("/links")
        .bearer(&token)
        .json(&json!({"url": "https://github.com/makhmudov/RustyApi"}))
        .await;
    assert_eq!(created.status(), 201, "{}", created.text());
    let link = created.json::<Link>();
    assert_eq!(link.code.len(), 7);
    assert_eq!(link.clicks, 0);

    for _ in 0..3 {
        let hop = api.get(format!("/r/{}", link.code)).await;
        assert_eq!(hop.status(), 307);
        assert_eq!(
            hop.header("location"),
            Some("https://github.com/makhmudov/RustyApi")
        );
    }

    // Счётчик растёт в фоне после ответа — ждём, пока задачи отработают.
    let mut clicks = 0;
    for _ in 0..50 {
        clicks = api
            .get(format!("/links/{}", link.code))
            .bearer(&token)
            .await
            .json::<Link>()
            .clicks;
        if clicks == 3 {
            break;
        }
        rustyapi::tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(clicks, 3);

    // Свой код и его занятость.
    let custom = api
        .post("/links")
        .bearer(&token)
        .json(&json!({"url": "https://example.com", "code": "rustyapi"}))
        .await;
    assert_eq!(custom.status(), 201);
    let taken = api
        .post("/links")
        .bearer(&token)
        .json(&json!({"url": "https://example.org", "code": "rustyapi"}))
        .await;
    assert_eq!(
        (taken.status_code(), taken.json::<Value>()),
        (409, json!({"detail": "Code already taken"}))
    );
    assert_eq!(
        api.get("/links")
            .bearer(&token)
            .await
            .json::<Vec<Link>>()
            .len(),
        2
    );

    // После удаления редирект сразу перестаёт работать, хотя адрес был в кэше.
    let deleted = api.delete("/links/rustyapi").bearer(&token).await;
    assert_eq!(deleted.status(), 204);
    let gone = api.get("/r/rustyapi").await;
    assert_eq!(
        (gone.status_code(), gone.json::<Value>()),
        (404, json!({"detail": "Link not found"}))
    );
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn auth_ownership_and_validation() {
    let Some(app) = test_app("shortener_auth_test").await else {
        return;
    };
    let api = TestClient::new(app);
    assert_eq!(api.get("/links").await.status(), 401);

    let alice = sign_up(&api, "alice").await;
    let bob = sign_up(&api, "bob").await;
    let link = api
        .post("/links")
        .bearer(&alice)
        .json(&json!({"url": "https://example.com/secret"}))
        .await
        .json::<Link>();
    let url = format!("/links/{}", link.code);
    assert_eq!(api.get(&url).bearer(&bob).await.status(), 404);
    assert_eq!(api.delete(&url).bearer(&bob).await.status(), 404);
    assert_eq!(
        api.get("/links").bearer(&bob).await.json::<Value>(),
        json!([])
    );
    assert_eq!(api.get(&url).bearer(&alice).await.status(), 200);

    let me = api.get("/users/me").bearer(&bob).await.json::<Value>();
    assert_eq!(me["username"], "bob");
    assert!(me.get("password_hash").is_none());

    let invalid = api
        .post("/links")
        .bearer(&alice)
        .json(&json!({"url": "ftp://example.com", "code": "a"}))
        .await;
    assert_eq!(invalid.status(), 422);
    let detail = invalid.json::<Value>()["detail"].clone();
    assert_eq!(detail[0]["loc"], json!(["body", "url"]));
    assert_eq!(detail[1]["type"], "string_too_short");
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn health_reports_database() {
    let Some(app) = test_app("shortener_health_test").await else {
        return;
    };
    let ok = TestClient::new(app).get("/health").await;
    assert_eq!(
        (ok.status_code(), ok.json::<Value>()),
        (
            200,
            json!({"status": "ok", "database": "ok", "cache": "ok"})
        )
    );

    // База недоступна — 503, чтобы балансировщик снял экземпляр.
    let down = shortener::routes()
        .database(rustyapi::db::pool_from_url("postgres://nobody@127.0.0.1:1/none").unwrap())
        .cache(Cache::memory());
    let response = TestClient::new(down).get("/health").await;
    assert_eq!(response.status(), 503);
    assert_eq!(response.json::<Value>()["database"], "unavailable");
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn rate_limit_returns_429() {
    let app = shortener::routes().rate_limit(RateLimit::per_minute(2));
    let api = TestClient::new(app);
    assert_eq!(api.get("/").await.status(), 200);
    assert_eq!(api.get("/").await.status(), 200);
    let limited = api.get("/").await;
    assert_eq!(limited.status(), 429);
    assert!(limited.header("retry-after").is_some());
}

#[test]
fn settings_are_validated() {
    let env = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<Vec<_>>()
    };
    let settings: Settings = rustyapi::settings::settings_from(env(&[
        ("SECRET_KEY", "0123456789abcdef0123456789abcdef"),
        ("CORS_ORIGINS", "https://a.example,https://b.example"),
        ("TRUST_PROXY", "true"),
    ]))
    .unwrap();
    assert_eq!(settings.cors_origins.len(), 2);
    assert_eq!(settings.rate_limit_per_minute, 120);
    assert!(settings.trust_proxy);
    assert!(settings.migrations_dir.ends_with("/migrations"));

    let weak = rustyapi::settings::settings_from::<Settings, _>(env(&[("SECRET_KEY", "short")]));
    let message = weak.unwrap_err().to_string();
    assert!(message.contains("SECRET_KEY"), "{message}");
    assert!(rustyapi::settings::settings_from::<Settings, _>(env(&[])).is_err());
}

#[test]
fn openapi_documents_the_api() {
    let doc = shortener::routes().openapi();
    let paths = doc["paths"].as_object().unwrap();
    for path in [
        "/",
        "/health",
        "/users",
        "/token",
        "/users/me",
        "/links",
        "/links/{code}",
        "/r/{code}",
    ] {
        assert!(paths.contains_key(path), "{path}");
    }
    assert_eq!(
        doc["paths"]["/links"]["post"]["security"],
        json!([{"OAuth2PasswordBearer": []}])
    );
    assert!(
        doc["components"]["schemas"]["User"]["properties"]
            .get("password_hash")
            .is_none()
    );
}
