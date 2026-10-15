# PostgreSQL и ORM

Лёгкий слой над sqlx: пул, экстрактор `Db`, миграции при старте, `#[model]` с CRUD и построителями
запросов. Сырой SQL через `rustyapi::sqlx` доступен всегда.

## Подключение

```rust
#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    App::new()
        .database_from_env()          // DATABASE_URL, пул ленивый; DATABASE_POOL_SIZE (по умолчанию 10)
        .migrations("migrations")     // применить ./migrations/*.sql при старте
        .route(read_todo)
        .run()
        .await
}
```

Миграции — обычные файлы sqlx (`0001_init.sql` или пары `.up.sql`/`.down.sql`). Если `DATABASE_URL` не
задан, `run()` вернёт понятную ошибку. Готовый пул: `App::database(pool)`.

## Модель

```rust
use rustyapi::chrono::{DateTime, Utc};

#[model(table = "todos")]
struct Todo {
    #[primary_key]          // по умолчанию — поле `id`
    id: i64,
    title: String,
    done: bool,
    owner_id: i64,
    created_at: DateTime<Utc>,
}
```

`#[model]` = `#[schema]` (сериализация, валидация, OpenAPI) + свой `FromRow` + трейт `Model`
(`TABLE`, `PK`, `COLUMNS`). Колонки называются как поля. Поле с `#[serde(skip)]` (например,
`password_hash`) остаётся колонкой, но не попадает в JSON и OpenAPI.

## Запросы

```rust
#[get("/todos/{id}")]
async fn read_todo(db: Db, Path(id): Path<i64>) -> Result<Json<Todo>> {
    Ok(Json(Todo::get_or_404(&db, id).await?))      // 404 {"detail": "Todo not found"}
}

let list = Todo::select().eq("owner_id", user.id).order_by_desc("id").limit(20).all(&db).await?;
let new  = Todo::insert().set("title", &body.title).set("owner_id", user.id).execute(&db).await?;
Todo::update(id).set_opt("title", body.title).execute(&db).await?;   // PATCH: только переданные поля
Todo::delete(&db, id).await?;                                        // 404, если записи нет
let n = Todo::count(&db).await?;
```

| Метод | Что делает |
|---|---|
| `get`, `get_or_404`, `all`, `count`, `delete` | по первичному ключу / всей таблице |
| `select()` | `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `like`, `ilike`, `is_null`, `is_not_null`, `is_in`, `eq_opt`, `filter(col, op, v)`, `order_by`, `order_by_desc`, `limit`, `offset`, затем `all`, `first`, `first_or_404`, `count` |
| `insert()` | `set`, `set_opt`, `execute` → созданная запись (`RETURNING`) |
| `update(id)` | `set`, `set_opt`, `execute` → обновлённая запись или 404 |
| `delete_where()` | фильтры + `execute` → число строк (без условий запрещено) |

Первый аргумент — `&db`, `&pool`, `&mut *conn` или `&mut *tx`.

## Безопасность

* Имена колонок сверяются с `Model::COLUMNS` и берутся в кавычки; неизвестное имя → `DbError::UnknownColumn`
  до обращения к базе.
* Операторы `filter(col, op, v)` — только из белого списка `db::OPERATORS`.
* Значения всегда передаются параметрами (`$1`), никогда не вставляются в текст SQL.

Тест `rustyapi/tests/db.rs::column_injection_is_impossible` проверяет атаки через имя колонки и оператор.

## Ошибки

`DbError` (и `sqlx::Error` через `?`) превращаются в ответ:

| Ошибка | Ответ |
|---|---|
| нет записи (`get_or_404`, `update`, `delete`, `RowNotFound`) | 404 |
| unique violation (23505), foreign key (23503), exclusion (23P01) | 409 `{"detail": "Conflict"}` |
| остальное | 500 `{"detail": "Internal Server Error"}` + запись в лог |

## Транзакции

```rust
let todo = db
    .transaction(async |tx| {
        let owner = Owner::insert().set("name", "Азим").execute(&mut *tx).await?;
        Todo::insert().set("title", "...").set("owner_id", owner.id).execute(&mut *tx).await
    })
    .await?;
```

`Ok` — commit, `Err` — rollback. Нужен async-замыкание (`async |tx|`, Rust 1.85+).

## Тесты

Интеграционные тесты требуют `DATABASE_URL` (`docker compose up -d` в `examples/todo_api`) и без него
пропускаются. Каждый тест создаёт свою схему и задаёт `search_path` в URL
(`?options=-c%20search_path%3D<схема>`), поэтому тесты не мешают друг другу.
