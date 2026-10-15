//! Авторизация как в учебнике FastAPI «OAuth2 with Password and JWT».
//!
//! * [`hash_password`] / [`verify_password`] — argon2;
//! * [`create_access_token`] / [`decode_access_token`] — JWT HS256, ключ `SECRET_KEY`, срок
//!   `ACCESS_TOKEN_EXPIRE_MINUTES` (по умолчанию 30) из окружения;
//! * [`Auth`] — экстрактор `Authorization: Bearer <token>` (401 с `WWW-Authenticate: Bearer`);
//! * [`OAuth2PasswordRequestForm`], [`Token`] — форма входа и ответ, как в FastAPI;
//! * в OpenAPI — схема `OAuth2PasswordBearer`, поэтому кнопка **Authorize** в `/docs` работает.
//!
//! ```no_run
//! use rustyapi::prelude::*;
//! use rustyapi::auth::{self, Auth, OAuth2PasswordRequestForm, Token};
//!
//! #[post("/token")]
//! async fn login(form: OAuth2PasswordRequestForm) -> Result<Json<Token>> {
//!     // проверьте пользователя в базе: auth::verify_password(&form.password, &user.password_hash)
//!     Ok(Json(Token::bearer(auth::create_access_token(&form.username)?)))
//! }
//!
//! #[get("/me")]
//! async fn me(auth: Auth) -> Json<Value> {
//!     Json(json!({ "username": auth.sub() }))
//! }
//! ```

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::{Error, Result};
use crate::openapi::{OpenApiArg, Operation};
use crate::schema::Components;
use crate::{Form, HTTPException};

/// Имя схемы безопасности в OpenAPI (как у `OAuth2PasswordBearer` в FastAPI).
pub const SECURITY_SCHEME: &str = "OAuth2PasswordBearer";

// ---------------------------------------------------------------------------------------------
// Пароли
// ---------------------------------------------------------------------------------------------

/// Хэш пароля argon2id (формат PHC: `$argon2id$v=19$...`).
pub fn hash_password(password: &str) -> Result<String> {
    // 16 случайных байт из ОС (через uuid v4 → getrandom).
    let salt = SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes())
        .map_err(|e| Error::msg(format!("salt: {e}")))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| Error::msg(format!("hash_password: {e}")))
}

/// Проверяет пароль по хэшу из [`hash_password`]. Некорректный хэш — `false`.
pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

// ---------------------------------------------------------------------------------------------
// JWT
// ---------------------------------------------------------------------------------------------

/// Содержимое токена.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    /// Субъект — обычно id или имя пользователя.
    pub sub: String,
    /// Срок действия (unix-время, секунды).
    pub exp: i64,
    /// Время выпуска.
    #[serde(default)]
    pub iat: i64,
    /// Роли пользователя (для [`Auth::require_role`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Прочие поля.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

static SECRET_OVERRIDE: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Задаёт ключ подписи в коде (иначе берётся `SECRET_KEY` из окружения). Удобно для тестов.
pub fn set_secret_key(key: impl Into<String>) {
    if let Ok(mut slot) = SECRET_OVERRIDE.write() {
        *slot = Some(key.into());
    }
}

fn secret_key() -> Result<String> {
    if let Some(key) = SECRET_OVERRIDE.read().ok().and_then(|k| k.clone()) {
        return Ok(key);
    }
    match std::env::var("SECRET_KEY") {
        Ok(key) if !key.is_empty() => Ok(key),
        _ => Err(Error::msg(
            "не задан SECRET_KEY / SECRET_KEY is not set (сгенерируйте: openssl rand -hex 32)",
        )),
    }
}

fn expire_minutes() -> i64 {
    std::env::var("ACCESS_TOKEN_EXPIRE_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
}

/// Токен для субъекта на `ACCESS_TOKEN_EXPIRE_MINUTES` минут.
pub fn create_access_token(subject: impl ToString) -> Result<String> {
    create_access_token_with(subject, &[], None)
}

/// Токен с ролями и сроком (по умолчанию `ACCESS_TOKEN_EXPIRE_MINUTES`).
pub fn create_access_token_with(
    subject: impl ToString,
    roles: &[&str],
    expires_in: Option<chrono::Duration>,
) -> Result<String> {
    let now = chrono::Utc::now();
    let expires_in = expires_in.unwrap_or_else(|| chrono::Duration::minutes(expire_minutes()));
    encode_claims(&Claims {
        sub: subject.to_string(),
        exp: (now + expires_in).timestamp(),
        iat: now.timestamp(),
        roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        extra: Map::new(),
    })
}

/// Подписывает произвольные [`Claims`] (HS256, `SECRET_KEY`).
pub fn encode_claims(claims: &Claims) -> Result<String> {
    let key = secret_key()?;
    Ok(jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(key.as_bytes()),
    )?)
}

/// Проверяет подпись и срок. Ошибка — 401 «Could not validate credentials».
pub fn decode_access_token(token: &str) -> Result<Claims> {
    let key = secret_key()?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    jsonwebtoken::decode::<Claims>(
        token,
        &DecodingKey::from_secret(key.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|_| credentials_exception().into())
}

/// 401 «Could not validate credentials» с `WWW-Authenticate: Bearer` (как в учебнике FastAPI).
pub fn credentials_exception() -> HTTPException {
    HTTPException::new(StatusCode::UNAUTHORIZED, "Could not validate credentials")
        .with_header("WWW-Authenticate", "Bearer")
}

/// 401 «Not authenticated» — нет заголовка `Authorization: Bearer` (как `OAuth2PasswordBearer`).
pub fn not_authenticated() -> HTTPException {
    HTTPException::new(StatusCode::UNAUTHORIZED, "Not authenticated")
        .with_header("WWW-Authenticate", "Bearer")
}

// ---------------------------------------------------------------------------------------------
// Экстрактор
// ---------------------------------------------------------------------------------------------

/// Проверенный Bearer-токен из заголовка `Authorization`.
#[derive(Debug, Clone)]
pub struct Auth {
    claims: Claims,
    token: String,
}

impl Auth {
    /// Субъект токена (`sub`).
    pub fn sub(&self) -> &str {
        &self.claims.sub
    }

    /// Все поля токена.
    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// Сам токен.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Есть ли роль.
    pub fn has_role(&self, role: &str) -> bool {
        self.claims.roles.iter().any(|r| r == role)
    }

    /// 403 «Not enough permissions», если роли нет: `auth.require_role("admin")?`.
    pub fn require_role(&self, role: &str) -> Result<(), HTTPException> {
        if self.has_role(role) {
            Ok(())
        } else {
            Err(HTTPException::new(
                StatusCode::FORBIDDEN,
                "Not enough permissions",
            ))
        }
    }

    /// Добавляет в операцию требование авторизации (замок в `/docs`). Для своих зависимостей:
    /// `fn openapi(op, c) { Auth::document(op, c) }`.
    pub fn document(operation: &mut Operation, components: &mut Components) {
        <Self as OpenApiArg>::doc_arg(operation, components);
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Auth {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Response> {
        let token = bearer_token(parts).ok_or_else(|| not_authenticated().into_response())?;
        let claims = decode_access_token(&token).map_err(IntoResponse::into_response)?;
        Ok(Self { claims, token })
    }
}

/// Токен из `Authorization: Bearer <token>` (схема без учёта регистра, как в FastAPI).
fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty())
        .then(|| token.trim().to_owned())
}

impl OpenApiArg for Auth {
    fn doc_arg(operation: &mut Operation, _: &mut Components) {
        let requirement = json!({ SECURITY_SCHEME: [] });
        if !operation.security.contains(&requirement) {
            operation.security.push(requirement);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Форма входа и ответ
// ---------------------------------------------------------------------------------------------

/// Форма входа OAuth2 password flow (`application/x-www-form-urlencoded`), как в FastAPI.
#[crate::schema]
pub struct OAuth2PasswordRequestForm {
    /// Должно быть `password`, если передано.
    #[field(pattern = "^password$")]
    pub grant_type: Option<String>,
    /// Имя пользователя.
    pub username: String,
    /// Пароль.
    #[field(format = "password")]
    pub password: String,
    /// Запрошенные права через пробел.
    #[field(default = "")]
    pub scope: String,
    /// Идентификатор клиента.
    pub client_id: Option<String>,
    /// Секрет клиента.
    #[field(format = "password")]
    pub client_secret: Option<String>,
}

impl OAuth2PasswordRequestForm {
    /// `scope`, разбитый по пробелам.
    pub fn scopes(&self) -> Vec<&str> {
        self.scope.split_whitespace().collect()
    }
}

impl<S: Send + Sync> FromRequest<S> for OAuth2PasswordRequestForm {
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Response> {
        Form::<Self>::from_request(request, state)
            .await
            .map(Form::into_inner)
    }
}

impl OpenApiArg for OAuth2PasswordRequestForm {
    fn doc_arg(operation: &mut Operation, components: &mut Components) {
        <Form<Self> as OpenApiArg>::doc_arg(operation, components);
    }
}

/// Ответ на вход: `{"access_token": "...", "token_type": "bearer"}`.
#[crate::schema]
pub struct Token {
    /// JWT.
    pub access_token: String,
    /// Всегда `bearer`.
    pub token_type: String,
}

impl Token {
    /// `{"access_token": token, "token_type": "bearer"}`.
    pub fn bearer(access_token: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            token_type: "bearer".to_owned(),
        }
    }
}

/// Схема безопасности `OAuth2PasswordBearer` с адресом получения токена.
pub fn security_scheme(token_url: &str) -> crate::openapi::SecurityScheme {
    crate::openapi::SecurityScheme {
        name: SECURITY_SCHEME.to_owned(),
        scheme: json!({
            "type": "oauth2",
            "flows": {"password": {"scopes": {}, "tokenUrl": token_url}}
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords() {
        let hash = hash_password("secret").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("secret", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(!verify_password("secret", "not-a-hash"));
        assert_ne!(hash, hash_password("secret").unwrap(), "соль случайная");
    }
}
