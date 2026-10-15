//! Эталонный пример RustyAPI (JWT + PostgreSQL + CRUD). Наполняется по мере этапов ROADMAP.md.
//!
//! Этап 1: роуты, `APIRouter`, `HTTPException`, `status_code`.
//! Этап 2: схемы `#[schema]`, валидация `#[field(...)]`, ошибки 422 как в FastAPI.
//! Этап 3: OpenAPI, `/docs`, `/redoc`.
//! Этап 4: PostgreSQL, `#[model]`, миграции при старте.
//! Этап 5: пользователи, JWT-логин, `Depends<CurrentUser>`, роли; задачи видит только владелец.
//!
//! ```text
//! cd examples/todo_api
//! docker compose up -d
//! cp .env.example .env
//! cargo run
//! curl -X POST http://127.0.0.1:8000/users -H 'content-type: application/json' \
//!      -d '{"username":"azim","password":"supersecret"}'
//! curl -X POST http://127.0.0.1:8000/token -d 'username=azim&password=supersecret'
//! open http://127.0.0.1:8000/docs   # кнопка Authorize → логин → защищённые эндпоинты
//! ```

use rustyapi::auth::{self, OAuth2PasswordRequestForm, Token};
use rustyapi::chrono::{DateTime, Utc};
use rustyapi::depends::RequestCtx;
use rustyapi::openapi::Operation;
use rustyapi::prelude::*;
use rustyapi::schema::Components;

/// Пользователь. Хеш пароля хранится в базе, но никогда не попадает в ответы.
#[model(table = "users")]
struct User {
    #[primary_key]
    id: i64,
    username: String,
    #[serde(skip)]
    password_hash: String,
    /// `user` или `admin`.
    role: String,
    created_at: DateTime<Utc>,
}

/// Регистрация.
#[schema]
struct UserCreate {
    #[field(
        min_length = 3,
        max_length = 50,
        pattern = "^[a-zA-Z0-9_]+$",
        example = "azim"
    )]
    username: String,
    #[field(min_length = 8, max_length = 128, format = "password")]
    password: String,
}

/// Задача.
#[model(table = "todos")]
struct Todo {
    #[primary_key]
    id: i64,
    title: String,
    done: bool,
    created_at: DateTime<Utc>,
    owner_id: Option<i64>,
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

/// Текущий пользователь: токен из `Authorization: Bearer ...` → запись в базе.
///
/// Аналог `get_current_user` из учебника FastAPI. Вычисляется один раз на запрос.
#[derive(Clone)]
struct CurrentUser(User);

impl Dependency for CurrentUser {
    async fn resolve(ctx: &mut RequestCtx<'_>) -> rustyapi::Result<Self> {
        let auth: Auth = ctx.extract().await?;
        let db: Db = ctx.extract().await?;
        let id: i64 = auth
            .sub()
            .parse()
            .map_err(|_| auth::credentials_exception())?;
        match User::get(&db, id).await? {
            Some(user) => Ok(CurrentUser(user)),
            // Пользователя удалили, а токен ещё жив.
            None => Err(auth::credentials_exception().into()),
        }
    }

    fn openapi(operation: &mut Operation, components: &mut Components) {
        Auth::document(operation, components);
    }
}

/// Приветствие.
#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({ "message": "Hello from RustyAPI" }))
}

/// Регистрация пользователя.
#[post("/users", status_code = 201, tags = ["users"])]
async fn register(db: Db, Json(body): Json<UserCreate>) -> Result<Json<User>> {
    let user = User::insert()
        .set("username", body.username)
        .set("password_hash", auth::hash_password(&body.password)?)
        .execute(&db)
        .await
        .map_err(|e| match e.status_code().as_u16() {
            409 => HTTPException::new(409, "Username already registered").into(),
            _ => rustyapi::Error::from(e),
        })?;
    Ok(Json(user))
}

/// Логин: OAuth2 password flow (форма `username` + `password`), как в FastAPI.
#[post("/token", tags = ["users"])]
async fn login(db: Db, form: OAuth2PasswordRequestForm) -> Result<Json<Token>> {
    let user = User::select()
        .eq("username", form.username)
        .first(&db)
        .await?
        .filter(|user| auth::verify_password(&form.password, &user.password_hash))
        .ok_or_else(|| {
            HTTPException::new(401, "Incorrect username or password")
                .with_header("WWW-Authenticate", "Bearer")
        })?;
    let token = auth::create_access_token_with(user.id, &[user.role.as_str()], None)?;
    Ok(Json(Token::bearer(token)))
}

/// Текущий пользователь.
#[get("/users/me", tags = ["users"])]
async fn read_me(Depends(CurrentUser(user)): Depends<CurrentUser>) -> Json<User> {
    Json(user)
}

/// Все пользователи (только для роли `admin`).
#[get("/admin/users", tags = ["admin"])]
async fn list_users(db: Db, auth: Auth) -> Result<Json<Vec<User>>> {
    auth.require_role("admin")?;
    Ok(Json(User::select().order_by("id").all(&db).await?))
}

/// Мои задачи.
#[get("")]
async fn list_todos(
    db: Db,
    Depends(CurrentUser(user)): Depends<CurrentUser>,
    Query(filter): Query<TodoFilter>,
) -> Result<Json<Vec<Todo>>> {
    let todos = Todo::select()
        .eq("owner_id", user.id)
        .eq_opt("done", filter.done)
        .order_by_desc("id")
        .limit(filter.limit)
        .offset(filter.offset)
        .all(&db)
        .await?;
    Ok(Json(todos))
}

/// Одна задача по id. Чужая задача — 404, как будто её нет.
#[get("/{todo_id}")]
async fn read_todo(
    db: Db,
    Depends(CurrentUser(user)): Depends<CurrentUser>,
    Path(todo_id): Path<i64>,
) -> Result<Json<Todo>> {
    let todo = Todo::select()
        .eq("id", todo_id)
        .eq("owner_id", user.id)
        .first_or_404(&db)
        .await?;
    Ok(Json(todo))
}

/// Создать задачу.
#[post("", status_code = 201)]
async fn create_todo(
    db: Db,
    Depends(CurrentUser(user)): Depends<CurrentUser>,
    Json(body): Json<TodoCreate>,
) -> Result<Json<Todo>> {
    let todo = Todo::insert()
        .set("title", body.title)
        .set("owner_id", user.id)
        .execute(&db)
        .await?;
    Ok(Json(todo))
}

/// Изменить задачу (PATCH: только переданные поля).
#[patch("/{todo_id}")]
async fn update_todo(
    db: Db,
    Depends(CurrentUser(user)): Depends<CurrentUser>,
    Path(todo_id): Path<i64>,
    Json(body): Json<TodoUpdate>,
) -> Result<Json<Todo>> {
    let todo = Todo::update(todo_id)
        .eq("owner_id", user.id)
        .set_opt("title", body.title)
        .set_opt("done", body.done)
        .execute(&db)
        .await?;
    Ok(Json(todo))
}

/// Удалить задачу.
#[delete("/{todo_id}", status_code = status::HTTP_204_NO_CONTENT)]
async fn delete_todo(
    db: Db,
    Depends(CurrentUser(user)): Depends<CurrentUser>,
    Path(todo_id): Path<i64>,
) -> Result<()> {
    let deleted = Todo::delete_where()
        .eq("id", todo_id)
        .eq("owner_id", user.id)
        .execute(&db)
        .await?;
    if deleted == 0 {
        return Err(HTTPException::new(404, "Todo not found").into());
    }
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
        .route(register)
        .route(login)
        .route(read_me)
        .route(list_users)
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
    async fn test_pool(schema: &str) -> Option<rustyapi::db::Pool> {
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
        routes()
            .database(pool.clone())
            .run_migrations()
            .await
            .unwrap();
        Some(pool)
    }

    /// Запрос к приложению: JSON-тело или форма, необязательный токен.
    struct Call<'a> {
        pool: &'a rustyapi::db::Pool,
        token: Option<&'a str>,
    }

    impl Call<'_> {
        async fn send(&self, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
            let mut request = Request::builder().method(method).uri(uri);
            if let Some(token) = self.token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            let body = match body {
                Some(Value::String(form)) => {
                    request = request.header("content-type", "application/x-www-form-urlencoded");
                    Body::from(form)
                }
                Some(json) => {
                    request = request.header("content-type", "application/json");
                    Body::from(json.to_string())
                }
                None => Body::empty(),
            };
            auth::set_secret_key("todo-api-test-secret");
            let response = routes()
                .database(self.pool.clone())
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
    }

    /// Регистрирует пользователя и возвращает его токен.
    async fn sign_up(pool: &rustyapi::db::Pool, username: &str) -> String {
        let anon = Call { pool, token: None };
        let (status, user) = anon
            .send(
                "POST",
                "/users",
                Some(json!({"username": username, "password": "supersecret"})),
            )
            .await;
        assert_eq!(status, 201, "{user}");
        assert!(user.get("password_hash").is_none(), "хеш не утекает");
        let (status, token) = anon
            .send(
                "POST",
                "/token",
                Some(json!(format!("username={username}&password=supersecret"))),
            )
            .await;
        assert_eq!(status, 200, "{token}");
        token["access_token"].as_str().unwrap().to_owned()
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn crud_flow() {
        let Some(pool) = test_pool("todo_api_test").await else {
            return;
        };
        let token = sign_up(&pool, "azim").await;
        let api = Call {
            pool: &pool,
            token: Some(&token),
        };
        assert_eq!(
            api.send("GET", "/", None).await,
            (200, json!({"message": "Hello from RustyAPI"}))
        );
        let (status, me) = api.send("GET", "/users/me", None).await;
        assert_eq!(
            (status, &me["username"], &me["role"]),
            (200, &json!("azim"), &json!("user"))
        );

        let (status, todo) = api
            .send("POST", "/todos", Some(json!({"title": "Купить хлеб"})))
            .await;
        assert_eq!(status, 201);
        let id = todo["id"].as_i64().unwrap();
        assert_eq!(
            (&todo["done"], &todo["owner_id"]),
            (&json!(false), &me["id"])
        );

        let (status, todo) = api
            .send(
                "PATCH",
                &format!("/todos/{id}"),
                Some(json!({"done": true})),
            )
            .await;
        assert_eq!(
            (status, &todo["done"], &todo["title"]),
            (200, &json!(true), &json!("Купить хлеб"))
        );

        let (status, list) = api.send("GET", "/todos?done=true", None).await;
        assert_eq!((status, list.as_array().unwrap().len()), (200, 1));
        let (_, list) = api.send("GET", "/todos?done=false", None).await;
        assert_eq!(list, json!([]));

        assert_eq!(
            api.send("DELETE", &format!("/todos/{id}"), None).await,
            (204, Value::Null)
        );
        assert_eq!(
            api.send("GET", &format!("/todos/{id}"), None).await,
            (404, json!({"detail": "Todo not found"}))
        );
        assert_eq!(api.send("GET", "/todos/", None).await.0, 307);
        assert_eq!(
            api.send("GET", "/nope", None).await,
            (404, json!({"detail": "Not Found"}))
        );
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn auth_and_ownership() {
        let Some(pool) = test_pool("todo_api_auth_test").await else {
            return;
        };
        let anon = Call {
            pool: &pool,
            token: None,
        };
        // Без токена — 401 как в FastAPI.
        assert_eq!(
            anon.send("GET", "/todos", None).await,
            (401, json!({"detail": "Not authenticated"}))
        );

        let alice_token = sign_up(&pool, "alice").await;
        let bob_token = sign_up(&pool, "bob").await;
        let alice = Call {
            pool: &pool,
            token: Some(&alice_token),
        };
        let bob = Call {
            pool: &pool,
            token: Some(&bob_token),
        };

        // Повторная регистрация и неверный пароль.
        assert_eq!(
            anon.send(
                "POST",
                "/users",
                Some(json!({"username": "alice", "password": "supersecret"}))
            )
            .await,
            (409, json!({"detail": "Username already registered"}))
        );
        assert_eq!(
            anon.send(
                "POST",
                "/token",
                Some(json!("username=alice&password=wrongwrong"))
            )
            .await,
            (401, json!({"detail": "Incorrect username or password"}))
        );

        // Чужие задачи не видны и не меняются.
        let (_, todo) = alice
            .send("POST", "/todos", Some(json!({"title": "секрет"})))
            .await;
        let id = todo["id"].as_i64().unwrap();
        assert_eq!(bob.send("GET", "/todos", None).await, (200, json!([])));
        assert_eq!(bob.send("GET", &format!("/todos/{id}"), None).await.0, 404);
        assert_eq!(
            bob.send(
                "PATCH",
                &format!("/todos/{id}"),
                Some(json!({"done": true}))
            )
            .await
            .0,
            404
        );
        assert_eq!(
            bob.send("DELETE", &format!("/todos/{id}"), None).await.0,
            404
        );
        let (_, todo) = alice.send("GET", &format!("/todos/{id}"), None).await;
        assert_eq!(todo["done"], json!(false));

        // Роли: admin видит всех пользователей, обычный пользователь — 403.
        assert_eq!(
            bob.send("GET", "/admin/users", None).await,
            (403, json!({"detail": "Not enough permissions"}))
        );
        rustyapi::sqlx::query("UPDATE users SET role = 'admin' WHERE username = 'bob'")
            .execute(&pool)
            .await
            .unwrap();
        // Роль зашита в токен: нужен новый логин.
        let bob_admin = anon
            .send(
                "POST",
                "/token",
                Some(json!("username=bob&password=supersecret")),
            )
            .await
            .1["access_token"]
            .as_str()
            .unwrap()
            .to_owned();
        let admin = Call {
            pool: &pool,
            token: Some(&bob_admin),
        };
        let (status, users) = admin.send("GET", "/admin/users", None).await;
        assert_eq!((status, users.as_array().unwrap().len()), (200, 2));

        // Пользователя удалили — его токен больше не работает.
        rustyapi::sqlx::query("DELETE FROM users WHERE username = 'alice'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            alice.send("GET", "/users/me", None).await,
            (401, json!({"detail": "Could not validate credentials"}))
        );
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn validation_errors() {
        // Ошибки 422 возникают до обращения к базе: подойдёт ленивый пул без сервера.
        let pool = rustyapi::db::pool_from_url("postgres://nobody@127.0.0.1:1/none").unwrap();
        let anon = Call {
            pool: &pool,
            token: None,
        };
        let (status, body) = anon
            .send(
                "POST",
                "/users",
                Some(json!({"username": "a b", "password": "short"})),
            )
            .await;
        assert_eq!(status, 422);
        assert_eq!(body["detail"][0]["type"], "string_pattern_mismatch");
        assert_eq!(body["detail"][1]["loc"], json!(["body", "password"]));

        let (status, body) = anon.send("POST", "/token", Some(json!("username=a"))).await;
        assert_eq!(status, 422);
        assert_eq!(body["detail"][0]["loc"], json!(["body", "password"]));
    }

    #[test]
    fn openapi_has_models_and_security() {
        let doc = routes().openapi();
        assert!(doc["components"]["schemas"]["Todo"].is_object());
        assert!(
            doc["components"]["schemas"]["User"]["properties"]
                .get("password_hash")
                .is_none()
        );
        assert_eq!(
            doc["paths"]["/todos/{todo_id}"]["patch"]["operationId"],
            "update_todo_todos__todo_id__patch"
        );
        assert_eq!(
            doc["paths"]["/todos"]["get"]["security"],
            json!([{"OAuth2PasswordBearer": []}])
        );
        assert!(doc["paths"]["/users"]["post"].get("security").is_none());
        assert_eq!(
            doc["components"]["securitySchemes"]["OAuth2PasswordBearer"]["flows"]["password"]["tokenUrl"],
            "/token"
        );
    }
}
