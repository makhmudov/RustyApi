# OpenAPI, Swagger UI, ReDoc

`/openapi.json` (OpenAPI 3.1), `/docs` (Swagger UI) and `/redoc` work out of the box, like FastAPI.
Parameters come from `Path`/`Query` extractors, the request body from `Json<T>`/`Form<T>`, responses
from the return type (`Json<T>`, `Result<Json<T>>`, `(StatusCode, Json<T>)`, `()`), `summary` from the
function name in Title Case and `description` from the doc comment. `operationId` follows FastAPI
(`read_item_items__item_id__get`), and endpoints with inputs get a `422 HTTPValidationError` response.

Configure with `.docs_url()`, `.redoc_url()`, `.openapi_url()`, `.no_docs()`, `.no_redoc()`,
`.no_openapi()`; hide an endpoint with `include_in_schema = false`. `rustyapi/tests/openapi_parity.rs`
compares paths and schemas with the document FastAPI generates for the same app.
