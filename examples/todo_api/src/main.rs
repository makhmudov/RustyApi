//! Эталонный пример RustyAPI (JWT + PostgreSQL + CRUD). Наполняется по мере этапов ROADMAP.md.
//!
//! Этап 1: роуты, `APIRouter`, `HTTPException`, `status_code`.
//! Этап 2: схемы `#[schema]`, валидация `#[field(...)]`, ошибки 422 как в FastAPI.
//! Этап 3: OpenAPI, `/docs`, `/redoc`.
//! Этап 4: PostgreSQL, `#[model]`, миграции при старте.
//! Этап 5: пользователи, JWT-логин, `Depends<CurrentUser>`, роли; задачи видит только владелец.
//! Этап 7: настройки из окружения, CORS, gzip, `TestClient` в тестах.
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

/// Настройки из окружения и `.env` (`DATABASE_URL` и `SECRET_KEY` читают сами `App` и `auth`).
#[schema]
struct Settings {
    /// Источники фронтенда через запятую: `CORS_ORIGINS=http://localhost:5173`.
    #[field(default = Vec::new())]
    cors_origins: Vec<String>,
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
    let settings: Settings = rustyapi::settings()?;
    routes()
        .cors_origins(settings.cors_origins)
        .gzip()
        .database_from_env()
        .run()
        .await
}

#[cfg(test)]
mod tests {
    use rustyapi::testing::TestClient;

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

    fn client(pool: &rustyapi::db::Pool) -> TestClient {
        auth::set_secret_key("todo-api-test-secret");
        TestClient::new(routes().database(pool.clone()))
    }

    /// Регистрирует пользователя и возвращает его токен.
    async fn sign_up(client: &TestClient, username: &str) -> String {
        let user = client
            .post("/users")
            .json(&json!({"username": username, "password": "supersecret"}))
            .await;
        assert_eq!(user.status(), 201, "{}", user.text());
        assert!(
            user.json::<Value>().get("password_hash").is_none(),
            "хеш не утекает"
        );
        let token = client
            .post("/token")
            .form(&json!({"username": username, "password": "supersecret"}))
            .await;
        assert_eq!(token.status(), 200, "{}", token.text());
        token.json::<Token>().access_token
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn crud_flow() {
        let Some(pool) = test_pool("todo_api_test").await else {
            return;
        };
        let api = client(&pool);
        let token = sign_up(&api, "azim").await;
        assert_eq!(
            api.get("/").await.json::<Value>(),
            json!({"message": "Hello from RustyAPI"})
        );
        let me = api.get("/users/me").bearer(&token).await.json::<Value>();
        assert_eq!(
            (&me["username"], &me["role"]),
            (&json!("azim"), &json!("user"))
        );

        let created = api
            .post("/todos")
            .bearer(&token)
            .json(&json!({"title": "Купить хлеб"}))
            .await;
        assert_eq!(created.status(), 201);
        let todo = created.json::<Todo>();
        assert_eq!((todo.done, todo.owner_id), (false, me["id"].as_i64()));

        let patched = api
            .patch(format!("/todos/{}", todo.id))
            .bearer(&token)
            .json(&json!({"done": true}))
            .await
            .json::<Todo>();
        assert_eq!(
            (patched.done, patched.title.as_str()),
            (true, "Купить хлеб")
        );

        let done = api
            .get("/todos")
            .query(&json!({"done": true}))
            .bearer(&token)
            .await;
        assert_eq!(done.json::<Vec<Todo>>().len(), 1);
        let open = api.get("/todos?done=false").bearer(&token).await;
        assert_eq!(open.json::<Value>(), json!([]));

        let deleted = api
            .delete(format!("/todos/{}", todo.id))
            .bearer(&token)
            .await;
        assert_eq!(
            (deleted.status_code(), deleted.text()),
            (204, String::new())
        );
        let missing = api.get(format!("/todos/{}", todo.id)).bearer(&token).await;
        assert_eq!(
            (missing.status_code(), missing.json::<Value>()),
            (404, json!({"detail": "Todo not found"}))
        );
        assert_eq!(api.get("/todos/").await.status(), 307);
        assert_eq!(
            api.get("/nope").await.json::<Value>(),
            json!({"detail": "Not Found"})
        );
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn auth_and_ownership() {
        let Some(pool) = test_pool("todo_api_auth_test").await else {
            return;
        };
        let api = client(&pool);
        // Без токена — 401 как в FastAPI.
        let anon = api.get("/todos").await;
        assert_eq!(
            (anon.status_code(), anon.json::<Value>()),
            (401, json!({"detail": "Not authenticated"}))
        );

        let alice = sign_up(&api, "alice").await;
        let bob = sign_up(&api, "bob").await;

        // Повторная регистрация и неверный пароль.
        let again = api
            .post("/users")
            .json(&json!({"username": "alice", "password": "supersecret"}))
            .await;
        assert_eq!(
            (again.status_code(), again.json::<Value>()),
            (409, json!({"detail": "Username already registered"}))
        );
        let wrong = api
            .post("/token")
            .form(&json!({"username": "alice", "password": "wrongwrong"}))
            .await;
        assert_eq!(
            (wrong.status_code(), wrong.json::<Value>()),
            (401, json!({"detail": "Incorrect username or password"}))
        );

        // Чужие задачи не видны и не меняются.
        let todo = api
            .post("/todos")
            .bearer(&alice)
            .json(&json!({"title": "секрет"}))
            .await
            .json::<Todo>();
        let url = format!("/todos/{}", todo.id);
        assert_eq!(
            api.get("/todos").bearer(&bob).await.json::<Value>(),
            json!([])
        );
        assert_eq!(api.get(&url).bearer(&bob).await.status(), 404);
        let patch = api
            .patch(&url)
            .bearer(&bob)
            .json(&json!({"done": true}))
            .await;
        assert_eq!(patch.status(), 404);
        assert_eq!(api.delete(&url).bearer(&bob).await.status(), 404);
        assert!(!api.get(&url).bearer(&alice).await.json::<Todo>().done);

        // Роли: admin видит всех пользователей, обычный пользователь — 403.
        let forbidden = api.get("/admin/users").bearer(&bob).await;
        assert_eq!(
            (forbidden.status_code(), forbidden.json::<Value>()),
            (403, json!({"detail": "Not enough permissions"}))
        );
        rustyapi::sqlx::query("UPDATE users SET role = 'admin' WHERE username = 'bob'")
            .execute(&pool)
            .await
            .unwrap();
        // Роль зашита в токен: нужен новый логин.
        let admin = api
            .post("/token")
            .form(&json!({"username": "bob", "password": "supersecret"}))
            .await
            .json::<Token>()
            .access_token;
        let users = api.get("/admin/users").bearer(&admin).await;
        assert_eq!(users.json::<Vec<User>>().len(), 2);

        // Пользователя удалили — его токен больше не работает.
        rustyapi::sqlx::query("DELETE FROM users WHERE username = 'alice'")
            .execute(&pool)
            .await
            .unwrap();
        let gone = api.get("/users/me").bearer(&alice).await;
        assert_eq!(
            (gone.status_code(), gone.json::<Value>()),
            (401, json!({"detail": "Could not validate credentials"}))
        );
    }

    #[rustyapi::tokio::test(crate = "rustyapi::tokio")]
    async fn validation_errors() {
        // Ошибки 422 возникают до обращения к базе: подойдёт ленивый пул без сервера.
        let pool = rustyapi::db::pool_from_url("postgres://nobody@127.0.0.1:1/none").unwrap();
        let api = client(&pool);
        let response = api
            .post("/users")
            .json(&json!({"username": "a b", "password": "short"}))
            .await;
        assert_eq!(response.status(), 422);
        let body = response.json::<Value>();
        assert_eq!(body["detail"][0]["type"], "string_pattern_mismatch");
        assert_eq!(body["detail"][1]["loc"], json!(["body", "password"]));

        let response = api.post("/token").form(&json!({"username": "a"})).await;
        assert_eq!(response.status(), 422);
        assert_eq!(
            response.json::<Value>()["detail"][0]["loc"],
            json!(["body", "password"])
        );
    }

    #[test]
    fn settings_from_env() {
        let settings: Settings = rustyapi::settings::settings_from([(
            "CORS_ORIGINS".to_owned(),
            "http://localhost:5173,https://todo.example".to_owned(),
        )])
        .unwrap();
        assert_eq!(settings.cors_origins.len(), 2);
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
