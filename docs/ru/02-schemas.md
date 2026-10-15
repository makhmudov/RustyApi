# Схемы, валидация, экстракторы

`#[schema]` — аналог `pydantic.BaseModel`.

```rust
use rustyapi::prelude::*;

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

#[post("/items", status_code = 201)]
async fn create_item(Json(item): Json<ItemCreate>) -> Json<ItemCreate> {
    Json(item)
}
```

`#[schema]` добавляет `Serialize`/`Deserialize` (через `::rustyapi::serde`), `Debug` и `Clone`
(если их нет в вашем `#[derive]`) и реализует трейты `Validate` и `ApiSchema` (схема для OpenAPI).

## Правила полей

| Pydantic `Field(...)` | RustyAPI `#[field(...)]` | Применимо к |
|---|---|---|
| `min_length=1`, `max_length=100` | `min_length = 1`, `max_length = 100` | строкам (символы), `Vec`/`HashMap`/`HashSet` (элементы) |
| `gt=0`, `ge=0`, `lt=10`, `le=10` | `gt = 0`, `ge = 0`, `lt = 10`, `le = 10` | числам |
| `pattern=r"^[a-z]+$"` | `pattern = r"^[a-z]+$"` (проверяется при компиляции) | строкам |
| `EmailStr` | `email` | строкам |
| `HttpUrl` | `url` | строкам |
| `= 10`, `Field(default=10)` | `default = 10` | любому типу |
| `Field(examples=[...])` | `example = "..."` | любому типу (только OpenAPI) |
| `Field(title=..., description=...)` | `title = "..."`, `description = "..."` или doc-комментарий | любому типу |

* `Option<T>` — необязательное поле (как `T | None = None`); правила проверяются, только если значение есть.
* Вложенные `#[schema]`-типы, `Vec`, `Option`, `HashMap` проверяются рекурсивно: `loc` = `["body", "tags", 0, "name"]`.
* Ошибка правила указывает на поле при компиляции: `gt` у строки даст
  «`gt`/`ge`/`lt`/`le` применимы только к числам».
* Проверить значение вручную: `item.validate()?` → `ValidationError` (в ответе — 422).

## serde-атрибуты

Поддерживаются `#[serde(rename = "...")]`, `#[serde(rename_all = "camelCase")]` (все правила serde),
`#[serde(skip)]`, `#[serde(default)]`. Имена из `rename` используются и в `loc` ошибок, и в схеме.

## Строковые enum

```rust
#[schema]
#[serde(rename_all = "snake_case")]
enum Status {
    InProgress,
    Done,
}
```

Неверное значение даёт `{"type": "enum", "msg": "Input should be 'in_progress' or 'done'"}`.
Поддерживаются только unit-варианты (как `class Status(str, Enum)`).

## Generic-схемы

`#[schema] struct Page<T> { items: Vec<T>, total: u64 }` — в OpenAPI попадёт как `Page_Item_`
(так же называет Pydantic v2).

## Экстракторы

| FastAPI | RustyAPI | `loc` ошибок |
|---|---|---|
| `item: Item` (тело) | `Json(item): Json<Item>` | `["body", ...]` |
| `item_id: int` (путь) | `Path(item_id): Path<i64>`, `Path((a, b)): Path<(i64, String)>` | `["path", "item_id"]` |
| `q: str, limit: int = 10` | `Query(params): Query<Params>` (поля `#[schema]`-структуры) | `["query", "q"]` |
| `username: str = Form()` | `Form(login): Form<Login>` | `["body", "username"]` |

Для `Json`, `Query`, `Form` тип должен реализовать `Deserialize` + `Validate` — проще всего `#[schema]`.
Если аргумент не подходит, компилятор скажет: «`Json<Item>` нельзя использовать как аргумент обработчика».

## Ошибки 422

Формат совпадает с FastAPI / Pydantic v2:

```json
{"detail": [{"type": "string_too_short", "loc": ["body", "name"],
             "msg": "String should have at least 1 character", "input": "", "ctx": {"min_length": 1}}]}
```

Типы ошибок: `missing`, `string_type`, `string_too_short`, `string_too_long`, `string_pattern_mismatch`,
`too_short`, `too_long`, `greater_than`, `greater_than_equal`, `less_than`, `less_than_equal`,
`int_parsing`, `int_type`, `int_from_float`, `float_parsing`, `float_type`, `bool_parsing`, `bool_type`,
`list_type`, `dict_type`, `model_attributes_type`, `enum`, `url_parsing`, `url_scheme`, `value_error`
(email), `json_invalid`, `uuid_parsing`.

Совпадение проверяется тестом `rustyapi/tests/fastapi_parity.rs`: 28 кейсов сравниваются с ответами
настоящего FastAPI (`rustyapi/tests/fixtures/fastapi_422.py`).

### Отличия от Pydantic

* Ошибки разбора JSON (неверный тип, нет поля) — по одной за запрос: serde останавливается на первой.
  Ошибки правил `#[field]` собираются все сразу, как в Pydantic.
* Нет lax-режима: `"1.5"` (строка) не превращается в число, `true` — в `1.0`.
* `Query` не поддерживает списки (`?tag=a&tag=b`).
