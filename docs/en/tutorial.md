# Tutorial: a notes API in 30 minutes

We'll build a notes API with sign-up, login, PostgreSQL, docs and tests. You need Rust 1.85+ and
Docker (for PostgreSQL).

## 1. Install and create a project

```sh
cargo install rustyapi-cli   # the tez command
tez new notes
cd notes
docker compose up -d     # PostgreSQL from docker-compose.yml
tez dev                  # build, run, restart on changes
```

Open http://127.0.0.1:8000/docs: `GET /` and `GET /health` are already there. The layout:

```text
src/main.rs         startup (main → app().run())
src/lib.rs          app(): all routers and app settings
src/routers/        endpoints
src/models/         database tables
migrations/         SQL migrations
tests/              tests
.env                DATABASE_URL, SECRET_KEY, PORT
```

## 2. First endpoint

Add to `src/routers/root.rs`:

```rust
/// Greets by name.
#[get("/hello/{name}")]
async fn hello(Path(name): Path<String>) -> Json<Value> {
    Json(json!({ "message": format!("Hello, {name}!") }))
}
```

and register it in `router()`: `.route(hello)`. Save the file: `tez dev` rebuilds and restarts the
server within a couple of seconds. `GET /hello/Azim` returns `{"message": "Hello, Azim!"}`, and the
endpoint shows up in `/docs`.

## 3. Model and CRUD

```sh
tez generate crud note
tez migrate run
```

This creates `src/models/note.rs`, `src/routers/notes.rs`, a migration and a test, and `/docs` gets
five `/notes` endpoints. Let's add a `text` field with a length check. In `*_create_notes.up.sql`:

```sql
CREATE TABLE notes (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    text TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Recreate the table with `tez migrate revert && tez migrate run`. In `src/models/note.rs` add the field
to the model and the create schema:

```rust
pub struct Note {
    // ...
    pub text: String,
}

pub struct NoteCreate {
    #[field(min_length = 1, max_length = 200)]
    pub name: String,
    #[field(max_length = 10_000)]
    #[serde(default)]
    pub text: String,
}
```

and `.set("text", body.text)` in `create_note`. Sending `{"name": ""}` gets the same answer as FastAPI:

```json
{"detail": [{"type": "string_too_short", "loc": ["body", "name"],
  "msg": "String should have at least 1 character", "input": "", "ctx": {"min_length": 1}}]}
```

## 4. Users and login

The full version is in [examples/todo_api](../../examples/todo_api/src/main.rs). In short:

```rust
use rustyapi::auth::{self, OAuth2PasswordRequestForm, Token};

#[post("/token")]
async fn login(db: Db, form: OAuth2PasswordRequestForm) -> Result<Json<Token>> {
    let user = User::select()
        .eq("username", form.username)
        .first(&db)
        .await?
        .filter(|u| auth::verify_password(&form.password, &u.password_hash))
        .ok_or_else(|| HTTPException::new(401, "Incorrect username or password"))?;
    Ok(Json(Token::bearer(auth::create_access_token(user.id)?)))
}

#[get("/me")]
async fn me(auth: Auth) -> Json<Value> {
    Json(json!({ "user_id": auth.sub() }))
}
```

The Authorize button in `/docs` works right away: log in and protected endpoints are called with the
token.

## 5. Tests

```rust
#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn hello() {
    let client = common::client(None);
    let response = client.get("/hello/Azim").await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>()["message"], "Hello, Azim!");
}
```

`cargo test` runs everything; database tests use a separate schema in the `DATABASE_URL` database.

## 6. Frontend client and production

```sh
tez generate client --ts --out ../web/src/api.ts   # typed TypeScript client
tez routes                                         # all endpoints as a table
tez run                                            # release build
```

A Dockerfile and production settings are in [examples/production](../../examples/production).
