# Shortener — продакшен-пример RustyAPI

Сокращатель ссылок, на котором видно, как RustyAPI-сервис выглядит в продакшене:

| Что | Где |
|---|---|
| Настройки из окружения с валидацией (`SECRET_KEY` ≥ 32 символов и т. д.) | `src/settings.rs` |
| Регистрация, вход по JWT, ссылки видит только владелец | `src/users.rs`, `src/links.rs` |
| Редирект `GET /r/{code}` с кэшем в Redis; без Redis — в памяти; упавший Redis не ломает редиректы | `src/links.rs` |
| Счётчик переходов в фоновой задаче после ответа | `src/links.rs` |
| CORS, gzip, лимит тела (413), таймаут (504), rate limit по IP (429) | `src/lib.rs` |
| `/health` с проверкой базы и кэша (503 + таймаут 2 с) | `src/system.rs` |
| Миграции при старте, корректная остановка по SIGTERM | `src/lib.rs`, `src/main.rs` |
| Многоэтапный Dockerfile (непривилегированный пользователь, HEALTHCHECK) | `Dockerfile` |
| Postgres + Redis + приложение одной командой | `docker-compose.yml` |
| Интеграционные тесты на `TestClient` | `tests/api.rs` |

## Запуск в Docker

```bash
cd examples/production
docker compose up --build
open http://localhost:8000/docs
```

## Локально

```bash
cd examples/production
docker compose up -d postgres redis
cp .env.example .env
cargo run            # или: tez dev
```

## Попробовать

```bash
curl -X POST localhost:8000/users -H 'content-type: application/json' \
     -d '{"username":"azim","password":"supersecret"}'
TOKEN=$(curl -s -X POST localhost:8000/token -d 'username=azim&password=supersecret' | jq -r .access_token)
curl -X POST localhost:8000/links -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"url":"https://github.com/makhmudov/RustyApi","code":"rustyapi"}'
curl -i localhost:8000/r/rustyapi                                  # 307 → github.com
curl -H "authorization: Bearer $TOKEN" localhost:8000/links/rustyapi  # "clicks": 1
curl localhost:8000/health
```

## Тесты

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/shortener cargo test -p shortener
```

Без `DATABASE_URL` тесты с базой пропускаются.

## Переменные окружения

| Переменная | По умолчанию | Назначение |
|---|---|---|
| `DATABASE_URL` | — | PostgreSQL |
| `SECRET_KEY` | — (обязательна, ≥ 32 символов) | ключ подписи JWT: `openssl rand -hex 32` |
| `REDIS_URL` | нет → кэш в памяти | Redis для кэша редиректов |
| `ACCESS_TOKEN_EXPIRE_MINUTES` | 30 | срок жизни токена |
| `CORS_ORIGINS` | пусто | источники фронтенда через запятую |
| `RATE_LIMIT_PER_MINUTE` | 120 | запросов в минуту с одного IP |
| `REQUEST_TIMEOUT_SECS` | 10 | таймаут запроса |
| `BODY_LIMIT_BYTES` | 65536 | максимальный размер тела |
| `TRUST_PROXY` | false | брать IP из `X-Forwarded-For` (только за своим прокси) |
| `MIGRATIONS_DIR` | `./migrations` крейта | каталог миграций (в образе — `/app/migrations`) |
| `HOST` / `PORT` | 127.0.0.1 / 8000 | адрес сервера (в образе — 0.0.0.0) |
| `RUST_LOG` | info | уровень логов |

## Свой проект в Docker

Скопируйте `Dockerfile` в проект, созданный `tez new`, замените `-p shortener` на имя своего крейта
и уберите префикс `examples/production/` у пути к миграциям. Контекст сборки — корень проекта:
`docker build -t myapp .`. Перед продакшеном: свой `SECRET_KEY`, `TRUST_PROXY=true` только за
собственным reverse proxy, несколько экземпляров — общий Redis (rate limit при этом считается
в каждом экземпляре отдельно).
