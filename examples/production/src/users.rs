//! Пользователи и вход по OAuth2 password flow.

use rustyapi::auth::{self, OAuth2PasswordRequestForm, Token};
use rustyapi::chrono::{DateTime, Utc};
use rustyapi::prelude::*;

/// Пользователь. Хеш пароля в ответы не попадает.
#[model(table = "users")]
pub struct User {
    #[primary_key]
    pub id: i64,
    pub username: String,
    #[serde(skip)]
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Регистрация.
#[schema]
struct UserCreate {
    #[field(
        min_length = 3,
        max_length = 50,
        pattern = "^[a-zA-Z0-9_]+$",
        example = "azim"
    )]
    username: String,
    #[field(min_length = 8, max_length = 128, format = "password")]
    password: String,
}

/// id пользователя из токена — без похода в базу.
pub fn user_id(auth: &Auth) -> Result<i64> {
    auth.sub()
        .parse()
        .map_err(|_| auth::credentials_exception().into())
}

/// Регистрация пользователя.
#[post("/users", status_code = 201, tags = ["users"])]
async fn register(db: Db, Json(body): Json<UserCreate>) -> Result<Json<User>> {
    let user = User::insert()
        .set("username", body.username)
        .set("password_hash", auth::hash_password(&body.password)?)
        .execute(&db)
        .await
        .map_err(|e| match e.status_code().as_u16() {
            409 => HTTPException::new(409, "Username already registered").into(),
            _ => rustyapi::Error::from(e),
        })?;
    Ok(Json(user))
}

/// Вход: форма `username` + `password` → JWT.
#[post("/token", tags = ["users"])]
async fn login(db: Db, form: OAuth2PasswordRequestForm) -> Result<Json<Token>> {
    let user = User::select()
        .eq("username", form.username)
        .first(&db)
        .await?
        .filter(|user| auth::verify_password(&form.password, &user.password_hash))
        .ok_or_else(|| {
            HTTPException::new(401, "Incorrect username or password")
                .with_header("WWW-Authenticate", "Bearer")
        })?;
    Ok(Json(Token::bearer(auth::create_access_token(user.id)?)))
}

/// Текущий пользователь.
#[get("/users/me", tags = ["users"])]
async fn me(db: Db, auth: Auth) -> Result<Json<User>> {
    let user = User::get(&db, user_id(&auth)?)
        .await?
        .ok_or_else(auth::credentials_exception)?;
    Ok(Json(user))
}

pub fn router() -> APIRouter {
    APIRouter::new().route(register).route(login).route(me)
}
