# CLAUDE.md — RustyAPI

RustyAPI — веб-фреймворк на Rust в стиле FastAPI. Пользователь FastAPI должен понимать код без обучения. Девиз: «пишется как FastAPI, работает как Rust».

## Принципы

1. Синтаксис FastAPI. Имена и поведение копируем у FastAPI/Pydantic: `HTTPException`, `APIRouter`, `Depends`, `include_router`, `status_code=201`, `tags`, ошибки 422 в формате Pydantic v2, `{"detail": ...}`.
2. Одна зависимость. Пользователь пишет в Cargo.toml только `rustyapi`. Всё остальное реэкспортируется.
3. Сгенерированный макросами код ссылается только на `::rustyapi::...` (никогда на `::serde`, `::sqlx` и т.д.). Для serde — `#[serde(crate = "::rustyapi::serde")]`. Чужие derive-макросы, которые жёстко ссылаются на свой crate (sqlx::FromRow, utoipa::ToSchema, validator), НЕ используем — генерируем impl сами.
4. Понятные ошибки компиляции. Макросы выдают `syn::Error` с подсказкой на русском/английском.
5. Скорость разработки важнее микрооптимизаций (но без лишних аллокаций в горячем пути).

## Структура workspace

```
RustyApi/
├── Cargo.toml            # workspace + профили сборки
├── rustyapi/             # основной crate (рантайм)
├── rustyapi-macros/      # proc-macro: #[get], #[schema], #[model], #[main]
├── rustyapi-cli/         # бинарь `tez`: new / dev / run / migrate / routes
├── examples/todo_api/    # эталонный пример (JWT + PostgreSQL + CRUD)
└── docs/                 # документация (ru / uz / en)
```

## Стек (зафиксированные версии)

axum 0.8 · tokio 1 · tower-http 0.6 · serde/serde_json · serde_path_to_error · sqlx 0.8 (postgres, rustls) · jsonwebtoken 9 · argon2 0.5 · chrono · uuid · regex · dotenvy · envy · tracing · notify (CLI) · syn 2. Обновлять мажорные версии только отдельной задачей.

## Ключевые технические решения

* Роуты: `#[get("/items/{id}")]` превращает функцию в unit-struct (как actix), реализующий трейт `Endpoint` (method, path, `MethodRouter`, документация OpenAPI). Регистрация: `router.route(read_item)`.
* OpenAPI — свой генератор на serde_json, без utoipa. Трейт `ApiSchema` реализован для примитивов, String, Vec, Option, HashMap, chrono, uuid, Value и генерируется `#[schema]`. Вложенные типы — `$ref` в components.
* Автодокументация аргументов через autoref-специализацию: `(&&Probe::<T>::new()).__rustyapi_doc_arg(op)`. Проверено: вызов именно через `&&`. Неизвестные экстракторы молча пропускаются.
* Экстракторы `Json/Path/Query/Form` — свои newtype над axum: ошибки → 422 в формате FastAPI, путь ошибки через `serde_path_to_error`, затем `Validate`.
* Swagger UI / ReDoc — HTML с CDN jsdelivr (как FastAPI), без build-скриптов.
* ORM — лёгкий слой над sqlx: `#[model(table = "...")]` генерирует `FromRow` и CRUD; билдеры `select()/insert()/update()` на `PgArguments` с проверкой имён колонок.

## Команды

* `cargo build` / `cargo test --workspace`
* `cargo clippy --workspace -- -D warnings` — должно быть чисто
* Тесты БД: нужен `DATABASE_URL` (docker compose up -d в examples/todo_api)
* `cargo run -p rustyapi-cli -- dev` из папки примера — проверка автоперезапуска

## Правила работы

* Каждая фича: код + тест + пример в `examples/` + раздел в docs.
* Публичный API меняем только вместе с CHANGELOG.md.
* Перед коммитом: fmt, clippy, тесты.
* Тесты макросов — через `trybuild` (и успешные, и ожидаемые ошибки компиляции).
