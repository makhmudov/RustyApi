# Туториал: API заметок за 30 минут

Соберём API заметок с регистрацией, входом, PostgreSQL, документацией и тестами. Нужны Rust 1.85+ и
Docker (для PostgreSQL).

## 1. Установка и новый проект

```sh
cargo install --git https://github.com/makhmudov/RustyApi rustyapi-cli   # команда tez
tez new notes
cd notes
docker compose up -d     # PostgreSQL из docker-compose.yml
tez dev                  # сборка, запуск, перезапуск при изменениях
```

Откройте http://127.0.0.1:8000/docs: там уже есть `GET /` и `GET /health`. Проект устроен так:

```text
src/main.rs         запуск (main → app().run())
src/lib.rs          app(): все роутеры, настройки приложения
src/routers/        эндпоинты
src/models/         таблицы базы
migrations/         SQL-миграции
tests/              тесты
.env                DATABASE_URL, SECRET_KEY, PORT
```

## 2. Первый эндпоинт

Добавьте в `src/routers/root.rs`:

```rust
/// Приветствие по имени.
#[get("/hello/{name}")]
async fn hello(Path(name): Path<String>) -> Json<Value> {
    Json(json!({ "message": format!("Привет, {name}!") }))
}
```

и подключите его в `router()`: `.route(hello)`. Сохраните файл: `tez dev` пересоберёт и перезапустит
сервер за пару секунд. `GET /hello/Азим` ответит `{"message": "Привет, Азим!"}`, а эндпоинт появится
в `/docs`.

## 3. Модель и CRUD

```sh
tez generate crud note
tez migrate run
```

Появятся `src/models/note.rs`, `src/routers/notes.rs`, миграция и тест, а в `/docs` — пять
эндпоинтов `/notes`. Добавим поле `text` с проверкой длины. В миграции `*_create_notes.up.sql`:

```sql
CREATE TABLE notes (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    text TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Пересоздайте таблицу: `tez migrate revert && tez migrate run`. В `src/models/note.rs` добавьте поле
в модель и схему создания:

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

и в `create_note` — `.set("text", body.text)`. Если прислать `{"name": ""}`, ответ будет таким же,
как у FastAPI:

```json
{"detail": [{"type": "string_too_short", "loc": ["body", "name"],
  "msg": "String should have at least 1 character", "input": "", "ctx": {"min_length": 1}}]}
```

## 4. Пользователи и вход

Полный пример — в [examples/todo_api](../../examples/todo_api/src/main.rs). Коротко:

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

Кнопка Authorize в `/docs` работает сразу: введите логин и пароль, и защищённые эндпоинты будут
вызываться с токеном.

## 5. Тесты

```rust
#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn hello() {
    let client = common::client(None);
    let response = client.get("/hello/Азим").await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>()["message"], "Привет, Азим!");
}
```

`cargo test` запускает все тесты; тесты с базой используют отдельную схему в базе из `DATABASE_URL`.

## 6. Клиент для фронтенда и продакшен

```sh
tez generate client --ts --out ../web/src/api.ts   # типизированный TypeScript-клиент
tez routes                                         # все эндпоинты таблицей
tez run                                            # release-сборка
```

Dockerfile и настройки для продакшена — в [examples/production](../../examples/production).
