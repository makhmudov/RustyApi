# Schemas, validation, extractors

`#[schema]` is the counterpart of `pydantic.BaseModel`.

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

`#[schema]` adds `Serialize`/`Deserialize` (via `::rustyapi::serde`), `Debug` and `Clone` (unless you
derive them yourself) and implements `Validate` and `ApiSchema` (the OpenAPI schema).

## Field rules

| Pydantic `Field(...)` | RustyAPI `#[field(...)]` | Applies to |
|---|---|---|
| `min_length`, `max_length` | `min_length = 1`, `max_length = 100` | strings (chars), collections (items) |
| `gt`, `ge`, `lt`, `le` | `gt = 0`, ... | numbers |
| `pattern` | `pattern = r"^[a-z]+$"` (checked at compile time) | strings |
| `EmailStr` / `HttpUrl` | `email` / `url` | strings |
| default value | `default = 10` | any type |
| `examples`, `title`, `description` | `example = ...`, `title = "..."`, `description = "..."` or a doc comment | any type |

`Option<T>` fields are optional and their rules run only for `Some`. Nested schemas, `Vec`, `Option`
and maps are validated recursively. serde `rename`, `rename_all`, `skip` and `default` are honoured.
Unit-only enums become string enums.

## Extractors

`Json<T>` (loc `body`), `Path<T>` (loc `path`), `Query<T>` (loc `query`), `Form<T>` (loc `body`).
Errors are returned as `422 {"detail": [...]}` in the Pydantic v2 format (`type`, `loc`, `msg`, `input`,
`ctx`); `rustyapi/tests/fastapi_parity.rs` compares 28 cases against real FastAPI responses.

Differences: JSON decoding reports the first error only (field rules report all), there is no lax mode
(`"1.5"` is not a number), and `Query` does not support repeated keys.
