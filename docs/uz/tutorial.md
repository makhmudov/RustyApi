# Qo'llanma: 30 daqiqada eslatmalar API'si

Ro'yxatdan o'tish, kirish, PostgreSQL, hujjatlar va testlar bilan eslatmalar API'sini yig'amiz.
Rust 1.85+ va Docker (PostgreSQL uchun) kerak.

## 1. O'rnatish va yangi loyiha

```sh
cargo install --git https://github.com/makhmudov/RustyApi rustyapi-cli   # tez buyrug'i
tez new notes
cd notes
docker compose up -d     # docker-compose.yml'dagi PostgreSQL
tez dev                  # yig'ish, ishga tushirish, o'zgarishda qayta ishga tushirish
```

http://127.0.0.1:8000/docs ni oching: `GET /` va `GET /health` allaqachon bor. Loyiha tuzilishi:

```text
src/main.rs         ishga tushirish (main → app().run())
src/lib.rs          app(): barcha routerlar va ilova sozlamalari
src/routers/        endpointlar
src/models/         ma'lumotlar bazasi jadvallari
migrations/         SQL migratsiyalar
tests/              testlar
.env                DATABASE_URL, SECRET_KEY, PORT
```

## 2. Birinchi endpoint

`src/routers/root.rs` ga qo'shing:

```rust
/// Ism bo'yicha salomlashish.
#[get("/hello/{name}")]
async fn hello(Path(name): Path<String>) -> Json<Value> {
    Json(json!({ "message": format!("Salom, {name}!") }))
}
```

va uni `router()` ga ulang: `.route(hello)`. Faylni saqlang: `tez dev` serverni bir necha soniyada
qayta yig'ib, qayta ishga tushiradi. `GET /hello/Azim` `{"message": "Salom, Azim!"}` qaytaradi,
endpoint esa `/docs` da paydo bo'ladi.

## 3. Model va CRUD

```sh
tez generate crud note
tez migrate run
```

`src/models/note.rs`, `src/routers/notes.rs`, migratsiya va test yaratiladi, `/docs` da esa beshta
`/notes` endpointi paydo bo'ladi. Uzunlik tekshiruvi bilan `text` maydonini qo'shamiz.
`*_create_notes.up.sql` migratsiyasida:

```sql
CREATE TABLE notes (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    text TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Jadvalni qayta yarating: `tez migrate revert && tez migrate run`. `src/models/note.rs` da maydonni
modelga va yaratish sxemasiga qo'shing:

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

va `create_note` da `.set("text", body.text)`. `{"name": ""}` yuborilsa, javob FastAPI'dagi bilan
bir xil bo'ladi:

```json
{"detail": [{"type": "string_too_short", "loc": ["body", "name"],
  "msg": "String should have at least 1 character", "input": "", "ctx": {"min_length": 1}}]}
```

## 4. Foydalanuvchilar va kirish

To'liq misol [examples/todo_api](../../examples/todo_api/src/main.rs) da. Qisqacha:

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

`/docs` dagi Authorize tugmasi darhol ishlaydi: login va parolni kiriting, himoyalangan endpointlar
token bilan chaqiriladi.

## 5. Testlar

```rust
#[rustyapi::tokio::test(crate = "rustyapi::tokio")]
async fn hello() {
    let client = common::client(None);
    let response = client.get("/hello/Azim").await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>()["message"], "Salom, Azim!");
}
```

`cargo test` barcha testlarni ishga tushiradi; bazali testlar `DATABASE_URL` bazasidagi alohida
sxemadan foydalanadi.

## 6. Frontend uchun klient va production

```sh
tez generate client --ts --out ../web/src/api.ts   # turlangan TypeScript klient
tez routes                                         # barcha endpointlar jadvali
tez run                                            # release yig'ish
```

Dockerfile va production sozlamalari [examples/production](../../examples/production) da.
