# Dependencies and auth

## Depends

A dependency is a type implementing `Dependency`; handlers receive it as `Depends<T>`, like FastAPI:

```rust
use rustyapi::depends::RequestCtx;
use rustyapi::prelude::*;

#[derive(Clone)]
struct CurrentUser(User);

impl Dependency for CurrentUser {
    async fn resolve(ctx: &mut RequestCtx<'_>) -> rustyapi::Result<Self> {
        let auth: Auth = ctx.extract().await?;   // any extractor, Depends<...> included
        let db: Db = ctx.extract().await?;
        let id: i64 = auth.sub().parse().map_err(|_| rustyapi::auth::credentials_exception())?;
        let user = User::get(&db, id).await?.ok_or_else(rustyapi::auth::credentials_exception)?;
        Ok(CurrentUser(user))
    }

    // So /docs shows the lock icon on endpoints using this dependency.
    fn openapi(op: &mut rustyapi::openapi::Operation, c: &mut rustyapi::schema::Components) {
        Auth::document(op, c);
    }
}

#[get("/users/me")]
async fn read_me(Depends(CurrentUser(user)): Depends<CurrentUser>) -> Json<User> {
    Json(user)
}
```

The value is cached per request (FastAPI's `use_cache=True`). An error from `resolve` becomes the response.

## JWT and passwords

| FastAPI tutorial | RustyAPI |
|---|---|
| `pwd_context.hash(p)` / `verify` | `auth::hash_password(p)?` / `auth::verify_password(p, &hash)` (argon2) |
| `jwt.encode({"sub": ..., "exp": ...})` | `auth::create_access_token(user.id)?` or `create_access_token_with(id, &["admin"], None)?` |
| `OAuth2PasswordBearer(tokenUrl="token")` | the `Auth` extractor; `App::oauth2_token_url("/auth/token")` for another path |
| `OAuth2PasswordRequestForm = Depends()` | a `form: OAuth2PasswordRequestForm` argument |
| `{"access_token": t, "token_type": "bearer"}` | `Json(Token::bearer(t))` |

The signing key comes from `SECRET_KEY` (or `auth::set_secret_key`), the lifetime from
`ACCESS_TOKEN_EXPIRE_MINUTES` (default 30). Algorithm: HS256.

Responses match FastAPI: no token gives `401 {"detail": "Not authenticated"}`, a bad or expired one
`401 {"detail": "Could not validate credentials"}`, both with `WWW-Authenticate: Bearer`.

## Roles

Roles live in the token (`roles`). `auth.require_role("admin")?` answers `403 {"detail": "Not enough
permissions"}`; `auth.has_role("admin")` returns a `bool`. A role change takes effect on the next login.

## /docs

The `OAuth2PasswordBearer` scheme (password flow) is added to OpenAPI as soon as one endpoint requires
it. Swagger UI's Authorize button posts the form to `/token` and sends the token with each request.
See `examples/todo_api` for the full example.
