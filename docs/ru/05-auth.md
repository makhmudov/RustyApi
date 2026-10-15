# Зависимости и авторизация

## Depends

Зависимость — тип, реализующий `Dependency`. Обработчик получает её через `Depends<T>`, как в FastAPI:

```rust
use rustyapi::depends::RequestCtx;
use rustyapi::prelude::*;

#[derive(Clone)]
struct CurrentUser(User);

impl Dependency for CurrentUser {
    async fn resolve(ctx: &mut RequestCtx<'_>) -> rustyapi::Result<Self> {
        let auth: Auth = ctx.extract().await?;   // любой экстрактор, в том числе Depends<...>
        let db: Db = ctx.extract().await?;
        let id: i64 = auth.sub().parse().map_err(|_| rustyapi::auth::credentials_exception())?;
        let user = User::get(&db, id).await?.ok_or_else(rustyapi::auth::credentials_exception)?;
        Ok(CurrentUser(user))
    }

    // Чтобы /docs показывал замок у эндпоинтов с этой зависимостью.
    fn openapi(op: &mut rustyapi::openapi::Operation, c: &mut rustyapi::schema::Components) {
        Auth::document(op, c);
    }
}

#[get("/users/me")]
async fn read_me(Depends(CurrentUser(user)): Depends<CurrentUser>) -> Json<User> {
    Json(user)
}
```

Результат кэшируется на время запроса (`use_cache=True` в FastAPI): если `CurrentUser` нужен
обработчику и другой зависимости, он загрузится один раз. Ошибка из `resolve` сразу становится ответом.

## JWT и пароли

| FastAPI-учебник | RustyAPI |
|---|---|
| `pwd_context.hash(p)` / `verify` | `auth::hash_password(p)?` / `auth::verify_password(p, &hash)` (argon2) |
| `jwt.encode({"sub": ..., "exp": ...})` | `auth::create_access_token(user.id)?` или `create_access_token_with(id, &["admin"], None)?` |
| `OAuth2PasswordBearer(tokenUrl="token")` | экстрактор `Auth`; `App::oauth2_token_url("/auth/token")`, если путь другой |
| `OAuth2PasswordRequestForm = Depends()` | аргумент `form: OAuth2PasswordRequestForm` |
| `{"access_token": t, "token_type": "bearer"}` | `Json(Token::bearer(t))` |

Ключ подписи — `SECRET_KEY` из окружения (или `auth::set_secret_key` в коде), срок жизни —
`ACCESS_TOKEN_EXPIRE_MINUTES` (по умолчанию 30). Алгоритм HS256.

Ответы совпадают с FastAPI: без токена — `401 {"detail": "Not authenticated"}`, с плохим или
просроченным — `401 {"detail": "Could not validate credentials"}`, оба с `WWW-Authenticate: Bearer`.

## Роли

Роли лежат в токене (`roles`). `auth.require_role("admin")?` отвечает `403 {"detail": "Not enough
permissions"}`, `auth.has_role("admin")` возвращает `bool`. После смены роли нужен новый логин.

## /docs

Схема `OAuth2PasswordBearer` (password flow) попадает в OpenAPI, как только её требует хоть один
эндпоинт. Кнопка Authorize в Swagger UI отправляет форму на `/token` и подставляет токен в запросы.
Полный пример — `examples/todo_api`.
