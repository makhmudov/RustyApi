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
  - `rustyapi::prelude`; `Json`, `Path`, `Query`, `Form` пока реэкспортированы из axum.
