//! ORM на реальном PostgreSQL. Нужен `DATABASE_URL`; без него тесты, которым нужна база, пропускаются.
//! Каждый тест работает в своей схеме (search_path), чтобы не мешать параллельным тестам.

use http_body_util::BodyExt;
use rustyapi::axum::body::Body;
use rustyapi::axum::http::Request;
use rustyapi::chrono::{DateTime, Utc};
use rustyapi::db::{DbError, Pool};
use rustyapi::prelude::*;
use tower::ServiceExt;

#[model(table = "owners")]
struct Owner {
    id: i64,
    #[field(min_length = 1)]
    name: String,
}

#[model(table = "todos")]
struct Todo {
    #[primary_key]
    id: i64,
    title: String,
    done: bool,
    owner_id: i64,
    note: Option<String>,
    created_at: DateTime<Utc>,
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE owners (id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE todos (
    id BIGSERIAL PRIMARY KEY,
    title TEXT NOT NULL,
    done BOOLEAN NOT NULL DEFAULT FALSE,
    owner_id BIGINT NOT NULL REFERENCES owners(id),
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
"#;

/// Пул, у которого search_path — новая пустая схема `name`.
async fn pool_in_schema(name: &str) -> Option<Pool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL не задан — тест пропущен");
        return None;
    };
    let admin = rustyapi::db::pool_from_url(&url).unwrap();
    rustyapi::sqlx::raw_sql(&format!(
        "DROP SCHEMA IF EXISTS {name} CASCADE; CREATE SCHEMA {name};"
    ))
    .execute(&admin)
    .await
    .unwrap();
    let sep = if url.contains('?') { '&' } else { '?' };
    Some(
        rustyapi::db::pool_from_url(&format!("{url}{sep}options=-c%20search_path%3D{name}"))
            .unwrap(),
    )
}

async fn setup(name: &str) -> Option<Db> {
    let pool = pool_in_schema(name).await?;
    rustyapi::sqlx::raw_sql(SCHEMA_SQL)
        .execute(&pool)
        .await
        .unwrap();
    Some(Db(pool))
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn crud() {
    let Some(db) = setup("rustyapi_test_crud").await else {
        return;
    };

    let owner = Owner::insert()
        .set("name", "Азим")
        .execute(&db)
        .await
        .unwrap();
    let a = Todo::insert()
        .set("title", "Купить хлеб")
        .set("owner_id", owner.id)
        .execute(&db)
        .await
        .unwrap();
    assert!(!a.done);
    assert_eq!(a.note, None);
    let b = Todo::insert()
        .set("title", "Позвонить")
        .set("owner_id", owner.id)
        .set_opt("note", Some("вечером"))
        .execute(&db)
        .await
        .unwrap();

    assert_eq!(
        Todo::get(&db, a.id).await.unwrap().unwrap().title,
        "Купить хлеб"
    );
    assert_eq!(Todo::count(&db).await.unwrap(), 2);
    assert_eq!(Todo::all(&db).await.unwrap().len(), 2);

    let list = Todo::select()
        .eq("owner_id", owner.id)
        .order_by_desc("id")
        .limit(1)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, b.id);
    assert_eq!(
        Todo::select()
            .ilike("title", "%ХЛЕБ%")
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(Todo::select().is_null("note").count(&db).await.unwrap(), 1);
    assert_eq!(
        Todo::select()
            .is_in("id", vec![a.id, b.id])
            .count(&db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        Todo::select()
            .eq_opt("done", None::<bool>)
            .offset(1)
            .all(&db)
            .await
            .unwrap()
            .len(),
        1
    );

    // PATCH: меняются только переданные поля.
    let patched = Todo::update(a.id)
        .set_opt("title", None::<String>)
        .set_opt("done", Some(true))
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        (patched.title.as_str(), patched.done),
        ("Купить хлеб", true)
    );
    let unchanged = Todo::update(a.id).execute(&db).await.unwrap();
    assert!(unchanged.done);

    Todo::delete(&db, a.id).await.unwrap();
    let err = Todo::delete(&db, a.id).await.unwrap_err();
    assert_eq!(err.status_code(), 404);
    let err = Todo::get_or_404(&db, a.id).await.unwrap_err();
    assert_eq!(err.to_string(), "Todo not found");
    let err = Todo::update(a.id)
        .set("done", false)
        .execute(&db)
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), 404);

    // unique / foreign key → 409.
    let err = Owner::insert()
        .set("name", "Азим")
        .execute(&db)
        .await
        .unwrap_err();
    assert_eq!(
        (err.status_code(), err.code().as_deref()),
        (rustyapi::axum::http::StatusCode::CONFLICT, Some("23505"))
    );
    let err = Todo::insert()
        .set("title", "x")
        .set("owner_id", 999_i64)
        .execute(&db)
        .await
        .unwrap_err();
    assert_eq!(
        (err.status_code(), err.code().as_deref()),
        (rustyapi::axum::http::StatusCode::CONFLICT, Some("23503"))
    );
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn transactions() {
    let Some(db) = setup("rustyapi_test_tx").await else {
        return;
    };

    let id = db
        .transaction(async |tx| {
            let owner = Owner::insert().set("name", "tx").execute(&mut *tx).await?;
            Todo::insert()
                .set("title", "в транзакции")
                .set("owner_id", owner.id)
                .execute(&mut *tx)
                .await?;
            Ok::<_, DbError>(owner.id)
        })
        .await
        .unwrap();
    assert_eq!(Owner::get_or_404(&db, id).await.unwrap().name, "tx");

    let result: Result<(), DbError> = db
        .transaction(async |tx| {
            Owner::insert()
                .set("name", "откат")
                .execute(&mut *tx)
                .await?;
            Err(DbError::NotFound("отмена".into()))
        })
        .await;
    assert!(result.is_err());
    assert_eq!(
        Owner::select()
            .eq("name", "откат")
            .count(&db)
            .await
            .unwrap(),
        0
    );
}

/// Имена колонок проверяются по COLUMNS, операторы — по белому списку: SQL-инъекция невозможна.
#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn column_injection_is_impossible() {
    // Ошибка возникает до обращения к базе: подойдёт пул, который никуда не подключается.
    let db = Db(rustyapi::db::pool_from_url("postgres://nobody@127.0.0.1:1/none").unwrap());
    let attacks = [
        "id; DROP TABLE todos; --",
        "id = 1 OR 1=1 --",
        "\"id\"",
        "title\" = '' OR \"1\"=\"1",
        "ID",
    ];
    for column in attacks {
        let err = Todo::select().eq(column, 1_i64).all(&db).await.unwrap_err();
        assert!(
            matches!(err, DbError::UnknownColumn { .. }),
            "{column}: {err}"
        );
        let err = Todo::select().order_by(column).all(&db).await.unwrap_err();
        assert!(matches!(err, DbError::UnknownColumn { .. }));
        let err = Todo::insert()
            .set(column, 1_i64)
            .execute(&db)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::UnknownColumn { .. }));
        let err = Todo::update(1)
            .set(column, 1_i64)
            .execute(&db)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::UnknownColumn { .. }));
        let err = Todo::delete_where()
            .eq(column, 1_i64)
            .execute(&db)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::UnknownColumn { .. }));
    }
    for op in ["= 1 OR 1=1 --", "; DROP TABLE todos", "IN"] {
        let err = Todo::select()
            .filter("id", op, 1_i64)
            .all(&db)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::InvalidOperator(_)), "{op}");
    }
    // Значения всегда идут параметрами ($1), а не в текст SQL.
    let sql = Todo::select().eq("title", "'; DROP TABLE todos; --").sql();
    assert_eq!(
        sql,
        r#"SELECT "id", "title", "done", "owner_id", "note", "created_at" FROM "todos" WHERE "title" = $1"#
    );
    assert!(matches!(
        Todo::delete_where().execute(&db).await.unwrap_err(),
        DbError::Config(_)
    ));
}

/// Значение с кавычками и SQL внутри сохраняется как есть, таблица на месте.
#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn values_are_parameters() {
    let Some(db) = setup("rustyapi_test_values").await else {
        return;
    };
    let evil = "'); DROP TABLE todos; --";
    let owner = Owner::insert()
        .set("name", evil)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        Owner::select()
            .eq("name", evil)
            .first_or_404(&db)
            .await
            .unwrap()
            .id,
        owner.id
    );
    assert_eq!(Todo::count(&db).await.unwrap(), 0);
}

#[get("/owners/{owner_id}")]
async fn read_owner(db: Db, Path(owner_id): Path<i64>) -> Result<Json<Owner>> {
    Ok(Json(Owner::get_or_404(&db, owner_id).await?))
}

#[post("/owners", status_code = 201)]
async fn create_owner(db: Db, Json(owner): Json<Value>) -> Result<Json<Owner>> {
    let name = owner["name"].as_str().unwrap_or_default().to_owned();
    Ok(Json(Owner::insert().set("name", name).execute(&db).await?))
}

#[get("/raw")]
async fn raw(db: Db) -> Result<Json<i64>> {
    // Ошибка sqlx через `?` тоже превращается в ответ (здесь — 500).
    let (n,): (i64,) = rustyapi::sqlx::query_as("SELECT broken")
        .fetch_one(&*db)
        .await?;
    Ok(Json(n))
}

async fn call(app: App, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(json) => {
            req = req.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .into_router()
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        rustyapi::serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn http_errors() {
    let Some(db) = setup("rustyapi_test_http").await else {
        return;
    };
    let app = || {
        App::new()
            .database(db.pool().clone())
            .route(read_owner)
            .route(create_owner)
            .route(raw)
    };
    assert_eq!(
        call(app(), "GET", "/owners/1", None).await,
        (404, json!({"detail": "Owner not found"}))
    );
    let (status, owner) = call(app(), "POST", "/owners", Some(json!({"name": "a"}))).await;
    assert_eq!(status, 201);
    assert_eq!(
        call(app(), "GET", &format!("/owners/{}", owner["id"]), None)
            .await
            .0,
        200
    );
    assert_eq!(
        call(app(), "POST", "/owners", Some(json!({"name": "a"}))).await,
        (409, json!({"detail": "Conflict"}))
    );
    assert_eq!(
        call(app(), "GET", "/raw", None).await,
        (500, json!({"detail": "Internal Server Error"}))
    );
    // Без подключённой базы `Db` даёт 500, а не панику.
    assert_eq!(
        call(App::new().route(raw), "GET", "/raw", None).await.0,
        500
    );
}

#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn migrations_on_startup() {
    let Some(pool) = pool_in_schema("rustyapi_test_migrate").await else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("rustyapi_migrations_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("0001_owners.sql"),
        "CREATE TABLE owners (id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL UNIQUE);",
    )
    .unwrap();
    let app = App::new()
        .database(pool.clone())
        .migrations(dir.to_str().unwrap());
    app.run_migrations().await.unwrap();
    app.run_migrations().await.unwrap(); // повторный запуск ничего не ломает
    assert_eq!(Owner::count(&pool).await.unwrap(), 0);
    std::fs::remove_dir_all(dir).unwrap();

    let err = App::new()
        .migrations("migrations")
        .run_migrations()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("база не подключена"));
}
/// Футуры ORM — Send (иначе обработчик axum не скомпилируется).
fn _futures_are_send(db: Db) {
    fn is_send<T: Send>(_: T) {}
    is_send(async move {
        Owner::insert()
            .set("name", "x".to_owned())
            .execute(&db)
            .await
    });
}
