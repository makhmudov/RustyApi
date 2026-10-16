//! Зависимости и авторизация: Depends, Auth, роли, OAuth2-форма, OpenAPI.

use std::sync::atomic::{AtomicUsize, Ordering};

use http_body_util::BodyExt;
use rustyapi::auth::{self, OAuth2PasswordRequestForm, Token};
use rustyapi::axum::body::Body;
use rustyapi::axum::http::Request;
use rustyapi::depends::RequestCtx;
use rustyapi::prelude::*;
use tower::ServiceExt;

static RESOLVED: AtomicUsize = AtomicUsize::new(0);

/// Текущий пользователь из токена (в настоящем приложении — из базы).
#[derive(Clone)]
struct CurrentUser {
    name: String,
    admin: bool,
    /// Номер вычисления: одинаковый у всех потребителей в пределах запроса.
    resolution: usize,
}

impl Dependency for CurrentUser {
    async fn resolve(ctx: &mut RequestCtx<'_>) -> rustyapi::Result<Self> {
        let resolution = RESOLVED.fetch_add(1, Ordering::SeqCst);
        let auth: Auth = ctx.extract().await?;
        Ok(CurrentUser {
            name: auth.sub().to_owned(),
            admin: auth.has_role("admin"),
            resolution,
        })
    }

    fn openapi(op: &mut rustyapi::openapi::Operation, c: &mut rustyapi::schema::Components) {
        Auth::document(op, c);
    }
}

/// Зависимость от зависимости: использует CurrentUser (кэшируется на запрос).
#[derive(Clone)]
struct Greeting(String, usize);

impl Dependency for Greeting {
    async fn resolve(ctx: &mut RequestCtx<'_>) -> rustyapi::Result<Self> {
        let Depends(user) = ctx.extract::<Depends<CurrentUser>>().await?;
        Ok(Greeting(format!("Привет, {}", user.name), user.resolution))
    }
}

#[post("/token")]
async fn login(form: OAuth2PasswordRequestForm) -> Result<Json<Token>> {
    if form.password != "secret" {
        return Err(HTTPException::new(401, "Incorrect username or password")
            .with_header("WWW-Authenticate", "Bearer")
            .into());
    }
    let roles: &[&str] = if form.username == "admin" {
        &["admin"]
    } else {
        &[]
    };
    Ok(Json(Token::bearer(auth::create_access_token_with(
        &form.username,
        roles,
        None,
    )?)))
}

#[get("/users/me")]
async fn me(Depends(user): Depends<CurrentUser>, Depends(g): Depends<Greeting>) -> Json<Value> {
    Json(json!({
        "username": user.name, "admin": user.admin, "greeting": g.0,
        "cached": user.resolution == g.1,
    }))
}

#[get("/admin")]
async fn admin(auth: Auth) -> Result<Json<Value>> {
    auth.require_role("admin")?;
    Ok(Json(json!({"ok": true})))
}

#[get("/public")]
async fn public() -> Json<Value> {
    Json(json!({}))
}

fn app() -> App {
    auth::set_secret_key("test-secret-key");
    App::new().route(login).route(me).route(admin).route(public)
}

async fn send(req: Request<Body>) -> (u16, Option<String>, Value) {
    let response = app().into_router().oneshot(req).await.unwrap();
    let status = response.status().as_u16();
    let www = response
        .headers()
        .get("www-authenticate")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        www,
        rustyapi::serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn token_for(user: &str) -> String {
    let (status, _, body) = send(
        Request::post("/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "grant_type=password&username={user}&password=secret"
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["token_type"], "bearer");
    body["access_token"].as_str().unwrap().to_owned()
}

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut req = Request::get(uri);
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    req.body(Body::empty()).unwrap()
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn login_and_protected_endpoint() {
    let token = token_for("azim").await;
    let (status, _, body) = send(get("/users/me", Some(&token))).await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        // CurrentUser нужен обработчику и Greeting, но вычислен один раз за запрос.
        json!({"username": "azim", "admin": false, "greeting": "Привет, azim", "cached": true})
    );
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn unauthenticated_like_fastapi() {
    assert_eq!(
        send(get("/users/me", None)).await,
        (
            401,
            Some("Bearer".into()),
            json!({"detail": "Not authenticated"})
        )
    );
    assert_eq!(
        send(get("/users/me", Some("garbage"))).await,
        (
            401,
            Some("Bearer".into()),
            json!({"detail": "Could not validate credentials"})
        )
    );
    // Просроченный токен.
    app();
    let expired =
        auth::create_access_token_with("azim", &[], Some(rustyapi::chrono::Duration::minutes(-5)))
            .unwrap();
    assert_eq!(send(get("/users/me", Some(&expired))).await.0, 401);
    // Неверный пароль.
    let (status, www, body) = send(
        Request::post("/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("username=azim&password=wrong"))
            .unwrap(),
    )
    .await;
    assert_eq!((status, www.as_deref()), (401, Some("Bearer")));
    assert_eq!(body["detail"], "Incorrect username or password");
    // Нет полей формы — 422 как в FastAPI.
    let (status, _, body) = send(
        Request::post("/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("username=azim"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["detail"][0]["loc"], json!(["body", "password"]));
    // grant_type проверяется шаблоном.
    let (status, _, body) = send(
        Request::post("/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=client&username=a&password=secret"))
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, &body["detail"][0]["type"]),
        (422, &json!("string_pattern_mismatch"))
    );
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn roles() {
    let user = token_for("azim").await;
    let admin_token = token_for("admin").await;
    assert_eq!(
        send(get("/admin", Some(&user))).await,
        (403, None, json!({"detail": "Not enough permissions"}))
    );
    assert_eq!(send(get("/admin", Some(&admin_token))).await.0, 200);
}

#[test]
fn openapi_security_like_fastapi() {
    let doc = app().openapi();
    assert_eq!(
        doc["components"]["securitySchemes"],
        json!({"OAuth2PasswordBearer": {"type": "oauth2", "flows": {"password": {"scopes": {}, "tokenUrl": "/token"}}}})
    );
    assert_eq!(
        doc["paths"]["/users/me"]["get"]["security"],
        json!([{"OAuth2PasswordBearer": []}])
    );
    assert_eq!(
        doc["paths"]["/admin"]["get"]["security"],
        json!([{"OAuth2PasswordBearer": []}])
    );
    assert!(doc["paths"]["/public"]["get"].get("security").is_none());
    // Защищённый эндпоинт без параметров не получает 422 (как в FastAPI).
    assert!(
        doc["paths"]["/users/me"]["get"]["responses"]
            .get("422")
            .is_none()
    );
    let form = &doc["components"]["schemas"]["OAuth2PasswordRequestForm"];
    assert_eq!(form["required"], json!(["username", "password"]));
    assert_eq!(form["properties"]["password"]["format"], "password");
    assert_eq!(
        doc["paths"]["/token"]["post"]["requestBody"]["content"]["application/x-www-form-urlencoded"]
            ["schema"]["$ref"],
        "#/components/schemas/OAuth2PasswordRequestForm"
    );
    let custom = App::new()
        .oauth2_token_url("/auth/login")
        .route(me)
        .openapi();
    assert_eq!(
        custom["components"]["securitySchemes"]["OAuth2PasswordBearer"]["flows"]["password"]["tokenUrl"],
        "/auth/login"
    );
}
