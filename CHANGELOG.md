# Changelog

## [Unreleased]

### Added
- Cargo workspace: `rustyapi`, `rustyapi-macros`, `rustyapi-cli`, `examples/todo_api`.
- CI: fmt, clippy, тесты с PostgreSQL 16.
- Этап 1, ядро:
  - `App` (`title`, `version`, `description`, `route`, `include_router`, `run`, `serve`,
    `serve_with_shutdown`, `into_router`, `routes`); `run` слушает `HOST`/`PORT` (по умолчанию `127.0.0.1:8000`)
    и корректно останавливается по Ctrl+C/SIGTERM.
  - `APIRouter` (`prefix`, `tags`, `route`, `include_router`), трейт `Endpoint`, `RouteInfo`.
  - Макросы `#[get]`, `#[post]`, `#[put]`, `#[patch]`, `#[delete]`, `#[head]`, `#[options]` с параметрами
    `status_code`, `tags`, `summary`, `description`, `deprecated`; конвертер пути `{name:path}`.
  - `#[rustyapi::main]`.
  - `HTTPException` (`{"detail": ...}`, заголовки), `rustyapi::Error`, `rustyapi::Result`, модуль `status`.
  - 404/405 в формате FastAPI, `redirect_slashes` (307), паника → 500 JSON.
  - Логи в стиле uvicorn (`rustyapi::logging`).
  - `rustyapi::prelude`.
- Этап 2, схемы и валидация:
  - `#[schema]` для структур и строковых enum: `Serialize`/`Deserialize` через `::rustyapi::serde`,
    `Debug`/`Clone` (если не указаны), трейты `Validate` и `ApiSchema`; generic-схемы (`Page_Item_`).
  - Правила `#[field(...)]`: `min_length`, `max_length`, `gt`, `ge`, `lt`, `le`, `pattern`, `email`, `url`,
    `default`, `example`, `title`, `description`; serde `rename`/`rename_all`/`skip`/`default`.
  - Свои экстракторы `Json`, `Path`, `Query`, `Form` (вместо реэкспорта из axum): ошибки 422 в формате
    Pydantic v2 (`type`, `loc`, `msg`, `input`, `ctx`), путь через `serde_path_to_error`.
  - `ValidationError`, `ErrorDetail`, `LocItem`, `Components`; `Error::downcast_ref`.
  - Понятная ошибка компиляции, если аргумент обработчика не экстрактор.
  - `serde_json` с `preserve_order`: ключи JSON сохраняют порядок (как dict в Python).
- Этап 3, OpenAPI:
  - Свой генератор OpenAPI 3.1 (`rustyapi::openapi`), `App::openapi()`; схемы и пути совпадают с FastAPI
    для эталонного приложения.
  - `/openapi.json`, `/docs` (Swagger UI), `/redoc`, `/docs/oauth2-redirect`; `App::docs_url`, `redoc_url`,
    `openapi_url`, `no_docs`, `no_redoc`, `no_openapi`, `security_scheme`.
  - Параметр маршрута `include_in_schema = false`; `RouteInfo::openapi`, трейты `OpenApiArg`, `OpenApiResponse`.
  - `HashMap<String, Value>` в схеме — `additionalProperties: true`, как `dict` в Pydantic.
- Этап 4, PostgreSQL:
  - `rustyapi::db`: `pool_from_env`, `pool_from_url`, `migrate`, экстрактор `Db`, `transaction`, трейт `Conn`.
  - `App::database_from_env`, `database`, `migrations`, `run_migrations`, `db`.
  - `#[model(table = "...")]` и `#[primary_key]`: `#[schema]` + свой `FromRow` + трейт `Model`
    (`get`, `get_or_404`, `all`, `count`, `delete`, `select`, `insert`, `update`, `delete_where`).
  - Построители `Select`, `Insert`, `Update`, `Delete` на `PgArguments`: проверка колонок по `COLUMNS`,
    белый список операторов.
  - `DbError` → 404 / 409 / 500; `sqlx::Error` через `?` тоже превращается в ответ.
  - `examples/todo_api` на PostgreSQL: миграции, `docker-compose.yml`, `.env.example`.
- Этап 5, зависимости и авторизация:
  - `rustyapi::depends`: трейт `Dependency`, экстрактор `Depends<T>` с кэшем на запрос, `RequestCtx::extract`.
  - `rustyapi::auth`: `hash_password`/`verify_password` (argon2), `create_access_token`,
    `create_access_token_with` (роли, срок), `decode_access_token`, `set_secret_key`; `SECRET_KEY`,
    `ACCESS_TOKEN_EXPIRE_MINUTES` из окружения.
  - Экстрактор `Auth` (`sub`, `claims`, `has_role`, `require_role`), `OAuth2PasswordRequestForm`, `Token`.
  - OpenAPI: схема `OAuth2PasswordBearer` (password flow), `security` у защищённых операций — кнопка
    Authorize в `/docs` работает как в FastAPI; `App::oauth2_token_url`.
  - Правило `#[field(format = "...")]`; дополнительные условия у `Model::update(id)`.
  - `examples/todo_api`: пользователи, `/token`, `/users/me`, зависимость `CurrentUser`, задачи владельца,
    `/admin/users` для роли `admin`.
- Этап 6, CLI `tez`:
  - `tez new`: проект из шаблона (lib + bin, роутеры, модели, миграции, тесты, `.env`, docker-compose).
  - `tez dev`: сборка через `--message-format=json-render-diagnostics`, старый сервер работает во время
    сборки, SIGTERM → 5 с → kill, дебаунс 300 мс, ошибки сборки не останавливают `tez dev`;
    подсказка про mold.
  - `tez run`, `tez migrate new|run|revert|status`, `tez generate crud`, `tez routes`.
  - `App::run` понимает `RUSTYAPI_PRINT=routes|openapi`.
  - NOTICE от PostgreSQL не попадают в лог по умолчанию.
