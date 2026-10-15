# План разработки RustyAPI в Claude Code

Каждый этап — отдельная сессия Claude Code. Готовые промпты ниже: копируйте их как есть.
Правило: не переходить к следующему этапу, пока не выполнены критерии готовности.

---

## Этап 0. Подготовка проекта (1 день)
1. Создать репозиторий, положить в корень `CLAUDE.md` и `ROADMAP.md`.
2. В Claude Code: `/init` не нужен — CLAUDE.md уже есть.

**Промпт:**
> Прочитай CLAUDE.md. Создай Cargo workspace с crates rustyapi, rustyapi-macros, rustyapi-cli и examples/todo_api
> с зависимостями из раздела «Стек». Добавь GitHub Actions: fmt, clippy, test (с сервисом PostgreSQL 16).
> Пустые модули, чтобы `cargo build` проходил.

**Готово, когда:** `cargo build --workspace` и CI зелёные.

---

## Этап 1. Ядро: приложение, роуты, ошибки (3–5 дней)

**Цель:** работает минимальный «hello world» в стиле FastAPI.

```rust
#[get("/")]
async fn root() -> Json<Value> { Json(json!({"message": "Hello"})) }

#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    App::new().title("My API").route(root).run().await
}
```

**Промпт:**
> Этап 1 из ROADMAP.md. Реализуй: `App` (title, version, route, include_router, run на HOST/PORT,
> по умолчанию 127.0.0.1:8000, graceful shutdown по Ctrl+C/SIGTERM), `APIRouter` (prefix, tags),
> трейт `Endpoint`, макросы `#[get/post/put/patch/delete]` с параметрами status_code, tags, summary,
> deprecated, `#[rustyapi::main]`. `HTTPException` с ответом `{"detail": ...}`, 404/405 как в FastAPI,
> паника → 500 JSON. Лог запросов в стиле uvicorn. Тесты через trybuild для макросов.

**Готово, когда:** пример отвечает, неизвестный путь даёт `{"detail":"Not Found"}`, ошибка в макросе показывает понятное сообщение.

---

## Этап 2. Схемы, валидация, экстракторы (4–6 дней)

**Цель:** аналог Pydantic.

```rust
#[schema]
struct ItemCreate {
    #[field(min_length = 1, max_length = 100)]
    name: String,
    #[field(gt = 0)]
    price: f64,
    #[field(email)]
    owner_email: String,
    tags: Vec<String>,
    description: Option<String>,
}
```

**Промпт:**
> Этап 2. Атрибут `#[schema]`: добавляет Serialize/Deserialize (через `::rustyapi::serde`, `crate = ...`),
> Debug, Clone (если пользователь не указал сам), реализует `Validate` и `ApiSchema`.
> Правила `#[field(...)]` с именами как в Pydantic: min_length, max_length, gt, ge, lt, le, pattern, email,
> url, default, example. Option-поля проверяются только если Some. Вложенные схемы валидируются рекурсивно.
> Экстракторы Json, Path, Query, Form → ошибки 422 в формате Pydantic v2 (type, loc, msg, ctx),
> путь через serde_path_to_error, «missing field» → `{"type":"missing","msg":"Field required"}`.
> Поддержать serde rename / rename_all / skip. Enum из unit-вариантов → строковый enum.

**Готово, когда:** ответы на невалидные данные совпадают с FastAPI для тех же моделей (сравнить вручную на 5 кейсах).

---

## Этап 3. OpenAPI + Swagger (3–4 дня)

**Промпт:**
> Этап 3. Свой генератор OpenAPI 3.1 на serde_json (без utoipa). Документация аргументов через
> autoref-специализацию (`&&Probe<T>`), как описано в CLAUDE.md. Path-параметры из шаблона пути,
> Query-параметры из полей схемы, requestBody из Json<T>, ответы из возвращаемого типа
> (Json<T>, Result<T>, Vec, Option). summary — имя функции в Title Case, description — doc-комментарий.
> 422 HTTPValidationError как в FastAPI. Эндпоинты /openapi.json, /docs (Swagger UI с CDN), /redoc.
> operationId в формате FastAPI.

**Готово, когда:** /docs открывается, все схемы видны, «Try it out» работает.

---

## Этап 4. База данных PostgreSQL + ORM (5–7 дней)

```rust
#[model(table = "todos")]
struct Todo {
    #[primary_key] id: i64,
    title: String,
    done: bool,
    owner_id: i64,
    created_at: DateTime<Utc>,
}

let todo = Todo::get_or_404(&db, id).await?;
let list = Todo::select().eq("owner_id", user.id).order_by_desc("id").limit(20).all(&db).await?;
let new  = Todo::insert().set("title", &body.title).set("owner_id", user.id).execute(&db).await?;
Todo::update(id).set_opt("title", body.title).execute(&db).await?;   // PATCH: только переданные поля
Todo::delete(&db, id).await?;
```

**Промпт:**
> Этап 4. Модуль db: пул sqlx из DATABASE_URL, экстрактор `Db` (Deref на PgPool), автозапуск
> миграций из ./migrations при старте (опция App). Макрос `#[model(table)]` = `#[schema]` + свой impl
> FromRow (через `::rustyapi::sqlx`) + трейт Model (TABLE, PK, COLUMNS) + методы get, get_or_404, all,
> select, insert, update, delete, count. Билдеры на PgArguments, имена колонок проверяются по COLUMNS,
> операторы из белого списка. `DbError` → HTTPException: RowNotFound=404, unique violation=409,
> foreign key=409, остальное=500 с логом. Транзакции: `db.transaction(|tx| async { ... })`.
> Интеграционные тесты на реальном PostgreSQL.

**Готово, когда:** CRUD примера работает, SQL-инъекция через имя колонки невозможна (тест).

---

## Этап 5. Авторизация и зависимости (3–4 дня)

**Промпт:**
> Этап 5. Трейт `Dependency` + `Depends<T>` (аналог FastAPI Depends) с `RequestCtx::extract()`.
> Модуль auth: hash_password/verify_password (argon2), create_access_token/decode (jsonwebtoken, HS256,
> SECRET_KEY и ACCESS_TOKEN_EXPIRE_MINUTES из env), экстрактор `Auth` (Bearer, 401 с WWW-Authenticate),
> `OAuth2PasswordRequestForm`, `Token`. В OpenAPI — схема OAuth2 password flow, чтобы кнопка
> Authorize в /docs работала как в FastAPI. Пример: зависимость CurrentUser, загружающая User из БД.
> Роли: `Auth::require_role("admin")`.

**Готово, когда:** в /docs можно залогиниться и вызвать защищённый эндпоинт.

---

## Этап 6. CLI `tez` и автоперезапуск (4–5 дней)

| Команда | Что делает |
|---|---|
| `tez new myapp` | создаёт проект: main.rs, routers/, models/, migrations/, .env, docker-compose (Postgres), тесты |
| `tez dev` | сборка + запуск + слежение за src/, migrations/, .env; пересборка при изменениях |
| `tez run` | release-сборка и запуск |
| `tez migrate new <имя>` / `run` / `revert` | миграции |
| `tez generate crud <name>` | модель + роутер + миграция + тест |
| `tez routes` | таблица всех эндпоинтов |

**Промпт:**
> Этап 6. rustyapi-cli на clap + notify. `tez dev`: `cargo build --message-format=json-render-diagnostics`,
> путь к бинарю берётся из артефакта; старый сервер продолжает работать во время сборки, затем
> SIGTERM (таймаут 5 с → kill) и запуск нового. Дебаунс 300 мс, игнор target/ и .git/.
> При ошибке компиляции — красивый вывод и ожидание следующего изменения. Подсказка про mold/lld
> для ускорения линковки. Шаблоны для `tez new` и `tez generate crud`.

**Готово, когда:** изменение файла → сервер перезапущен за секунды, ошибки сборки не роняют `tez dev`.

---

## Этап 7. Батарейки (по 1–2 дня на пункт)
- CORS (`cors_allow_all`, `cors_origins`), gzip, лимит тела, таймауты
- Статика: `app.mount("/static", "static")`
- Настройки: `let s: Settings = rustyapi::settings()?` (.env + переменные окружения)
- `TestClient` как в FastAPI: `client.post("/items").json(&body).bearer(token).send().await`
- Фоновые задачи (`BackgroundTasks`), WebSocket (`#[websocket("/ws")]`)
- Загрузка файлов (`UploadFile`), Redis-кэш (feature flag), rate limiting
- Генерация TypeScript-клиента: `tez generate client --ts` (из openapi.json)

---

## Этап 8. Качество и релиз (1–2 недели)
1. Бенчмарки против FastAPI, Axum, Actix (wrk / oha): hello world, JSON, запрос в БД.
2. Документация: «FastAPI → RustyAPI» (таблица соответствий), туториал на ru/uz/en.
3. Пример продакшен-проекта + Dockerfile.
4. Публикация 0.1.0 на crates.io, анонс (Telegram-сообщества, Reddit r/rust, Habr).

---

## Как работать с Claude Code эффективно
- Один этап — одна сессия; начинайте с «Прочитай CLAUDE.md и ROADMAP.md, этап N».
- Просите сначала план (`Shift+Tab` → plan mode), потом реализацию.
- После каждого этапа: «обнови CLAUDE.md, если появились новые решения», и коммит.
- Сложные места (макросы, OpenAPI) — просите тесты trybuild до реализации.
- Держите `examples/todo_api` всегда рабочим — это главный интеграционный тест.

**Общий срок MVP (этапы 0–6):** ~5–7 недель при работе по вечерам.
