# Документация RustyAPI

Документация на трёх языках: русский, узбекский, английский. Начните с туториала, а если вы пришли
из FastAPI, держите под рукой таблицу соответствий.

## Начало

| Раздел | ru | uz | en |
|---|---|---|---|
| Туториал: API заметок за 30 минут | [ru](ru/tutorial.md) | [uz](uz/tutorial.md) | [en](en/tutorial.md) |
| FastAPI → RustyAPI: таблица соответствий | [ru](ru/fastapi-to-rustyapi.md) | [uz](uz/fastapi-to-rustyapi.md) | [en](en/fastapi-to-rustyapi.md) |

## Справочник

| Раздел | ru | en |
|---|---|---|
| Ядро: приложение, роуты, ошибки | [ru/01-core.md](ru/01-core.md) | [en/01-core.md](en/01-core.md) |
| Схемы, валидация, экстракторы | [ru/02-schemas.md](ru/02-schemas.md) | [en/02-schemas.md](en/02-schemas.md) |
| OpenAPI, Swagger UI, ReDoc | [ru/03-openapi.md](ru/03-openapi.md) | [en/03-openapi.md](en/03-openapi.md) |
| PostgreSQL и ORM | [ru/04-database.md](ru/04-database.md) | [en/04-database.md](en/04-database.md) |
| Зависимости и авторизация | [ru/05-auth.md](ru/05-auth.md) | [en/05-auth.md](en/05-auth.md) |
| CLI `tez` | [ru/06-cli.md](ru/06-cli.md) | [en/06-cli.md](en/06-cli.md) |
| Батарейки: CORS, gzip, статика, настройки, TestClient, фоновые задачи, WebSocket, файлы, кэш, rate limit, TS-клиент | [ru/07-batteries.md](ru/07-batteries.md) | [en/07-batteries.md](en/07-batteries.md) |

## Примеры и цифры

* [`examples/todo_api`](../examples/todo_api) — JWT + CRUD в одном файле.
* [`examples/production`](../examples/production) — продакшен-проект: Redis, rate limit, healthcheck, Dockerfile.
* [Бенчмарки](../benchmarks/RESULTS.md) против axum, actix-web и FastAPI.
* [Выпуск версии](../RELEASE.md).
