# OpenAPI, Swagger UI, ReDoc

Как в FastAPI, документация появляется сама:

| Адрес | Что там |
|---|---|
| `/openapi.json` | схема OpenAPI 3.1 |
| `/docs` | Swagger UI (кнопка «Try it out» работает) |
| `/redoc` | ReDoc |
| `/docs/oauth2-redirect` | служебная страница OAuth2 для Swagger UI |

Схема собирается один раз при старте. Ресурсы Swagger UI и ReDoc грузятся с CDN jsdelivr, как в FastAPI.

## Что попадает в схему

| Источник | В OpenAPI |
|---|---|
| `Path<i64>`, `Path<(i64, String)>`, `Path<Params>` | `parameters` с `in: path` (имена из шаблона пути) |
| `Query<Params>` | по параметру `in: query` на каждое поле `#[schema]`-структуры |
| `Json<T>` | `requestBody` `application/json` |
| `Form<T>` | `requestBody` `application/x-www-form-urlencoded` |
| `-> Json<T>`, `-> Result<Json<T>>`, `-> (StatusCode, Json<T>)` | схема ответа (`Vec`, `Option`, `HashMap` внутри `Json` тоже) |
| `-> String`, `-> &'static str`, `-> Html<..>` | `text/plain`, `text/html` |
| `-> ()`, `status_code = 204` | ответ без тела |
| имя функции | `summary` в Title Case (`read_item` → `Read Item`), если не задан `summary = "..."` |
| doc-комментарий | `description` |
| — | `operationId` как в FastAPI: `read_item_items__item_id__get` |
| есть параметры или тело | ответ `422` со схемой `HTTPValidationError` |

Неизвестные типы аргументов и ответов (свои экстракторы, `impl IntoResponse`) не ломают сборку: аргумент
пропускается, ответ описывается как `{}` (как FastAPI без `response_model`).

## Настройки

```rust
App::new()
    .title("My API")
    .version("1.0.0")
    .description("Описание")
    .docs_url("/swagger")       // по умолчанию /docs
    .redoc_url("/api/redoc")    // по умолчанию /redoc
    .openapi_url("/api/openapi.json");
// .no_docs(), .no_redoc(), .no_openapi() — как docs_url=None и т.д.
```

Скрыть эндпоинт: `#[get("/internal", include_in_schema = false)]`.
Схему можно получить в коде: `app.openapi()` (`serde_json::Value`).

## Совпадение с FastAPI

`rustyapi/tests/openapi_parity.rs` сравнивает `paths` и все `components.schemas` с документом, который
FastAPI строит для того же приложения (`tests/fixtures/fastapi_openapi.py`). Отличия:

* форма называется по типу (`Login`), а не `Body_login_login_post`;
* `-> String` описан как `text/plain` (так он и отдаётся), FastAPI пишет `application/json`.
