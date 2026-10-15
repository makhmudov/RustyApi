# RustyAPI

**Пишется как FastAPI, работает как Rust.** Веб-фреймворк на Rust для тех, кто привык к FastAPI:
декораторы-роуты, схемы с валидацией, автоматическая документация `/docs`, ошибки 422 в том же
формате — и в 5–7 раз больше запросов в секунду на том же железе.

[Документация](docs/README.md) · [Туториал](docs/ru/tutorial.md) · [FastAPI → RustyAPI](docs/ru/fastapi-to-rustyapi.md) ·
[Бенчмарки](benchmarks/RESULTS.md) · [Прод-пример](examples/production) · [English](#english)

```rust
use rustyapi::prelude::*;

/// Новый товар.
#[schema]
struct Item {
    #[field(min_length = 1, max_length = 100, example = "Ноутбук")]
    name: String,
    #[field(gt = 0)]
    price: f64,
    tags: Vec<String>,
}

/// Параметры поиска: `?q=...&limit=...`.
#[schema]
struct Search {
    q: Option<String>,
    #[field(default = 10, ge = 1, le = 100)]
    limit: i64,
}

#[get("/items/{item_id}")]
async fn read_item(Path(item_id): Path<i64>, Query(search): Query<Search>) -> Json<Value> {
    Json(json!({ "item_id": item_id, "q": search.q, "limit": search.limit }))
}

#[post("/items", status_code = 201)]
async fn create_item(Json(item): Json<Item>) -> Json<Item> {
    Json(item)
}

#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    App::new().route(read_item).route(create_item).run().await
}
```

```text
$ cargo run
INFO:     RustyAPI running on http://127.0.0.1:8000 (Press CTRL+C to quit)

$ curl -X POST localhost:8000/items -H 'content-type: application/json' -d '{"name":"","price":-1,"tags":[]}'
{"detail":[{"type":"string_too_short","loc":["body","name"],"msg":"String should have at least 1 character","input":"","ctx":{"min_length":1}},
           {"type":"greater_than","loc":["body","price"],"msg":"Input should be greater than 0","input":-1,"ctx":{"gt":0.0}}]}
```

Swagger UI — на `/docs`, ReDoc — на `/redoc`, схема — на `/openapi.json`.

## Что внутри

- **Роуты как в FastAPI:** `#[get]`, `#[post]`, …, `APIRouter` с префиксами и тегами, `HTTPException`,
  `status_code`, `Path`/`Query`/`Json`/`Form`.
- **Схемы и валидация:** `#[schema]` + `#[field(min_length, ge, pattern, format = "email", …)]`,
  ошибки 422 в формате pydantic v2, все ошибки сразу.
- **OpenAPI 3.1** из кода: Swagger UI, ReDoc, кнопка Authorize.
- **PostgreSQL:** `#[model]` поверх sqlx — `select().eq().order_by()`, `insert().set()`, `update`,
  `delete_where`, транзакции, миграции при старте; можно писать чистый SQL.
- **Зависимости и авторизация:** `Depends<T>` с кэшем на запрос, JWT, argon2, OAuth2 password flow,
  роли.
- **Батарейки:** CORS, gzip, статика, лимит тела, таймауты, настройки из окружения, `TestClient`,
  фоновые задачи, WebSocket, загрузка файлов, кэш (память / Redis), rate limit, TypeScript-клиент.
- **CLI `tez`:** `tez new`, `tez dev` (перезапуск при изменениях, старый сервер живёт при ошибке
  сборки), `tez migrate`, `tez generate crud`, `tez generate client --ts`, `tez routes`.

## Производительность

Один и тот же API на четырёх фреймворках, 4 ядра, `oha`, 64 соединения
([методика и код](benchmarks/README.md)):

| Фреймворк | hello world | JSON + валидация | запрос в PostgreSQL |
|---|---|---|---|
| **RustyAPI** | **121 454 rps** | **120 084 rps** | **20 965 rps** |
| axum | 143 620 rps | 130 000 rps | 23 163 rps |
| actix-web | 175 459 rps | 169 421 rps | 12 499 rps |
| FastAPI (4 воркера, uvloop) | 22 520 rps | 17 817 rps | 6 182 rps |

В 5,4 / 6,7 / 3,4 раза быстрее FastAPI; 85–92% от чистого axum, на котором построен RustyAPI.

## Установка

```bash
cargo install rustyapi-cli   # команда tez
tez new myapp && cd myapp
docker compose up -d   # PostgreSQL
tez dev
```

Или вручную в `Cargo.toml`:

```toml
[dependencies]
rustyapi = "0.1"
```

Нужен Rust 1.85+.

## Документация

| | ru | uz | en |
|---|---|---|---|
| Туториал: API за 30 минут | [ru](docs/ru/tutorial.md) | [uz](docs/uz/tutorial.md) | [en](docs/en/tutorial.md) |
| FastAPI → RustyAPI | [ru](docs/ru/fastapi-to-rustyapi.md) | [uz](docs/uz/fastapi-to-rustyapi.md) | [en](docs/en/fastapi-to-rustyapi.md) |
| Все разделы | [docs/README.md](docs/README.md) | | |

Примеры: [`examples/todo_api`](examples/todo_api) — JWT + CRUD в одном файле;
[`examples/production`](examples/production) — сокращатель ссылок с Redis, rate limit, healthcheck и Docker.

## English

RustyAPI is a Rust web framework that feels like FastAPI: route attributes, validated schemas,
automatic OpenAPI docs at `/docs`, FastAPI-compatible 422 errors, dependency injection, JWT auth,
a PostgreSQL ORM on top of sqlx and the `tez` CLI — while serving 5–7× more requests per second
than FastAPI. Start with the [tutorial](docs/en/tutorial.md) or the
[FastAPI → RustyAPI](docs/en/fastapi-to-rustyapi.md) cheat sheet.

## Лицензия

MIT или Apache-2.0 на ваш выбор.
