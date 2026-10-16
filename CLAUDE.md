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

axum 0.8 · tokio 1 · tower-http 0.6 · serde/serde_json (preserve_order) · serde_path_to_error · serde_urlencoded · sqlx 0.8 (postgres, rustls) · jsonwebtoken 9 · argon2 0.5 · chrono · uuid · regex · dotenvy · envy · tracing · notify (CLI) · syn 2. Обновлять мажорные версии только отдельной задачей.

## Ключевые технические решения

* Роуты: `#[get("/items/{id}")]` превращает функцию в unit-struct (как actix), реализующий трейт `Endpoint` (`info() -> RouteInfo`, `method_router()`). Исходная функция вкладывается в `method_router()` под именем `__rustyapi_handler_<name>`. Регистрация: `router.route(read_item)`.
* `status_code` эндпоинта — `route_layer` на его `MethodRouter`, меняет только ответ 200. Не `layer`: у слоёного MethodRouter появляется fallback, и `merge` двух методов одного пути паникует.
* `APIRouter::into_axum` объединяет методы одного пути в один `MethodRouter` и ставит ему fallback 405 JSON (заголовок `Allow` axum добавляет сам). 404 и `redirect_slashes` — fallback всего роутера; для редиректа свой матчер шаблонов путей (`PathMatcher`), обработчики не вызываются.
* Слои `App::into_router`: снаружи access-лог, затем `CatchPanicLayer` (паника → 500 JSON), затем роуты.
* `HTTPException` хранит данные в `Box`, чтобы `Result<T, HTTPException>` был маленьким (clippy::result_large_err в пользовательском коде).
* Ошибки: `rustyapi::Error` — как `anyhow::Error` (blanket `From<E: std::error::Error>`, сам `Error` не реализует). В ответе `HTTPException` достаётся через downcast, остальное — 500 + лог. `rustyapi::Result<T, E = Error>`.
* `{name:path}` (Starlette) переводится в `{*name}` (axum) при сборке роутера; в `RouteInfo.path` остаётся синтаксис FastAPI.
* Логи: свой `FormatEvent` (`LEVEL:` + отступ до 9 символов), access-лог с target `rustyapi::access`. `logging::init()` не трогает уже установленный пользователем subscriber.
* OpenAPI — свой генератор на serde_json, без utoipa. Трейт `ApiSchema` реализован для примитивов, String, Vec, Option, HashMap, chrono, uuid, Value и генерируется `#[schema]`. Вложенные типы — `$ref` в components.
* Автодокументация аргументов через autoref-специализацию: `(&&Probe::<T>::new()).__rustyapi_doc_arg(op)`. Проверено: вызов именно через `&&`. Неизвестные экстракторы молча пропускаются.
* Экстракторы `Json/Path/Query/Form` — свои newtype над axum: ошибки → 422 в формате FastAPI, путь ошибки через `serde_path_to_error`, затем `Validate`. Быстрый путь — разбор сразу в `T`; только при ошибке тело перечитывается в `Value`, чтобы заполнить `input` (исходное значение, как в Pydantic).
* `#[schema]` на структурах ставит `#[serde(remote = "Self")]` и сам реализует `Serialize`/`Deserialize` поверх inherent-функций: `Deserialize` принимает только map (serde по умолчанию принял бы JSON-массив, Pydantic — нет).
* Правила `#[field]` проверяются через трейты `HasLength`/`Numeric`/`AsStr` с `#[diagnostic::on_unimplemented]` (понятная ошибка «gt применимо только к числам»); вложенная валидация и схемы полей — autoref-специализация (`Probe`/`SchemaProbe`), типы без `Validate`/`ApiSchema` молча пропускаются.
* `Option`-поля определяются синтаксически (`Option<...>`): правила проверяются только для `Some`.
* Route-макрос проверяет каждый аргумент через `assert_extractor` — вместо «Handler is not implemented» компилятор называет конкретный аргумент.
* Паритет 422 с FastAPI: `rustyapi/tests/fastapi_parity.rs` + эталон `tests/fixtures/fastapi_422.{py,json}` (перегенерировать скриптом при изменении кейсов).
* Swagger UI / ReDoc — HTML с CDN jsdelivr (как FastAPI), без build-скриптов.
* Route-макрос кладёт в `RouteInfo::openapi` вложенную `fn doc(op, components)`: для каждого аргумента `(&&Probe::<T>::new()).__rustyapi_doc_arg(..)`, для возвращаемого типа `__rustyapi_doc_response`. Типы с `impl Trait` пропускаются (их нельзя подставить в `Probe::<T>`).
* `Query<T>` раскладывается на параметры по полям схемы; сама схема `T` в components не попадает (как в FastAPI).
* Паритет OpenAPI: `rustyapi/tests/openapi_parity.rs` + `tests/fixtures/fastapi_openapi.{py,json}`; числа сравниваются как f64.
* Проверка `/docs` в браузере: CDN jsdelivr в песочнице может быть закрыт — подставлять swagger-ui-dist из npm через `page.route` в Playwright.
* ORM — лёгкий слой над sqlx: `#[model(table = "...")]` генерирует `FromRow` и CRUD; билдеры `select()/insert()/update()` на `PgArguments` с проверкой имён колонок.
* CRUD — методы по умолчанию трейта `Model` (в макросе только константы и `FromRow`). Ошибки билдера (колонка, оператор, encode) копятся и возвращаются из `execute/all`, без паник.
* Исполнитель — свой трейт `db::Conn<'c>` (`&Db`, `&PgPool`, `&mut PgConnection`, `&mut Transaction`), а не реализация `sqlx::Executor` для `Db`.
* Методы, принимающие `impl Conn<'c>`, возвращают `impl Future + Send` явно, а не `async fn`: иначе в обработчике axum всплывает ошибка «implementation of `Conn` is not general enough» (HRTB + auto-trait Send).
* `Db` кладётся в extensions слоем `Extension` в `App::into_router`; без базы экстрактор отвечает 500 с логом.
* Транзакции — `AsyncFnOnce(&mut Transaction)` (async closures, Rust 1.85).
* Тесты БД: своя схема на тест через `?options=-c%20search_path%3D<схема>` в URL; без `DATABASE_URL` тест пропускается.

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
