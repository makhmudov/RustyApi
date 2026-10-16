# Бенчмарки

Одни и те же три эндпоинта на RustyAPI, axum, actix-web и FastAPI:

| Сценарий | Что проверяет |
|---|---|
| `GET /` | hello world: роутинг и сериализация JSON |
| `POST /items` | разбор JSON, валидация (`min_length`, `max_length`, `gt`), ответ |
| `GET /todos/{id}` | запрос в PostgreSQL по случайному id из 1000 строк |

Результаты: [RESULTS.md](RESULTS.md).

```sh
cd benchmarks
cargo build --release
pip install fastapi uvicorn uvloop httptools asyncpg
cargo install oha
createdb bench
DATABASE_URL=postgres://postgres:postgres@localhost:5432/bench python3 run.py
```

`BENCH_DURATION=30s BENCH_CONNECTIONS=256` меняют длительность и число соединений. Каталог —
отдельный cargo workspace, чтобы actix-web не попадал в сборку фреймворка.
