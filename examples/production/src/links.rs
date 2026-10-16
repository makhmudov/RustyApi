//! Короткие ссылки: создание, список, статистика, удаление и сам редирект.

use std::time::Duration;

use rustyapi::axum::response::Redirect;
use rustyapi::chrono::{DateTime, Utc};
use rustyapi::prelude::*;

use crate::users::user_id;

/// Сколько адрес ссылки живёт в кэше.
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// Сколько ждать кэш. Недоступный Redis не должен тормозить редиректы: дальше — сразу в базу.
const CACHE_TIMEOUT: Duration = Duration::from_millis(300);

/// Операция с кэшем, которая не может сломать запрос: ошибка или таймаут → `None` и предупреждение.
async fn best_effort<T>(what: &str, op: impl Future<Output = Result<T>>) -> Option<T> {
    match rustyapi::tokio::time::timeout(CACHE_TIMEOUT, op).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(e)) => {
            rustyapi::tracing::warn!("cache {what} failed: {e}");
            None
        }
        Err(_) => {
            rustyapi::tracing::warn!("cache {what} timed out");
            None
        }
    }
}

/// Короткая ссылка.
#[model(table = "links")]
pub struct Link {
    #[primary_key]
    pub id: i64,
    /// Код в адресе `/r/{code}`.
    pub code: String,
    pub url: String,
    /// Сколько раз по ссылке перешли.
    pub clicks: i64,
    pub owner_id: i64,
    pub created_at: DateTime<Utc>,
}

/// Новая ссылка.
#[schema]
struct LinkCreate {
    /// Куда вести.
    #[field(
        max_length = 2048,
        pattern = "^https?://\\S+$",
        example = "https://github.com/makhmudov/RustyApi"
    )]
    url: String,
    /// Свой код; не задан — сгенерируется случайный.
    #[field(
        min_length = 3,
        max_length = 32,
        pattern = "^[a-zA-Z0-9_-]+$",
        example = "rustyapi"
    )]
    code: Option<String>,
}

/// Постраничный список.
#[schema]
struct Page {
    #[field(default = 50, ge = 1, le = 500)]
    limit: i64,
    #[field(default = 0, ge = 0)]
    offset: i64,
}

fn cache_key(code: &str) -> String {
    format!("link:{code}")
}

/// Случайный код из 7 символов base62 (~3.5·10¹² вариантов).
fn random_code() -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut n = rustyapi::uuid::Uuid::new_v4().as_u128();
    (0..7)
        .map(|_| {
            let c = ALPHABET[(n % 62) as usize] as char;
            n /= 62;
            c
        })
        .collect()
}

/// Создать ссылку.
#[post("", status_code = 201)]
async fn create_link(db: Db, auth: Auth, Json(body): Json<LinkCreate>) -> Result<Json<Link>> {
    let owner = user_id(&auth)?;
    let custom = body.code.is_some();
    // Случайный код может совпасть с существующим — пробуем ещё раз.
    for _ in 0..5 {
        let code = body.code.clone().unwrap_or_else(random_code);
        let inserted = Link::insert()
            .set("code", code)
            .set("url", body.url.clone())
            .set("owner_id", owner)
            .execute(&db)
            .await;
        match inserted {
            Ok(link) => return Ok(Json(link)),
            Err(e) if e.status_code().as_u16() == 409 && custom => {
                return Err(HTTPException::new(409, "Code already taken").into());
            }
            Err(e) if e.status_code().as_u16() == 409 => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(HTTPException::new(503, "Could not generate a unique code, try again").into())
}

/// Мои ссылки, новые сначала.
#[get("")]
async fn list_links(db: Db, auth: Auth, Query(page): Query<Page>) -> Result<Json<Vec<Link>>> {
    let links = Link::select()
        .eq("owner_id", user_id(&auth)?)
        .order_by_desc("id")
        .limit(page.limit)
        .offset(page.offset)
        .all(&db)
        .await?;
    Ok(Json(links))
}

/// Ссылка со статистикой переходов. Чужая — 404.
#[get("/{code}")]
async fn read_link(db: Db, auth: Auth, Path(code): Path<String>) -> Result<Json<Link>> {
    let link = Link::select()
        .eq("code", code)
        .eq("owner_id", user_id(&auth)?)
        .first(&db)
        .await?
        .ok_or_else(|| HTTPException::new(404, "Link not found"))?;
    Ok(Json(link))
}

/// Удалить ссылку; редирект перестаёт работать сразу (запись в кэше удаляется).
#[delete("/{code}", status_code = status::HTTP_204_NO_CONTENT)]
async fn delete_link(db: Db, auth: Auth, cache: Cache, Path(code): Path<String>) -> Result<()> {
    let deleted = Link::delete_where()
        .eq("code", code.as_str())
        .eq("owner_id", user_id(&auth)?)
        .execute(&db)
        .await?;
    if deleted == 0 {
        return Err(HTTPException::new(404, "Link not found").into());
    }
    best_effort("delete", cache.delete(&cache_key(&code))).await;
    Ok(())
}

/// Переход по короткой ссылке: 307 на исходный адрес.
///
/// Адрес ищется в кэше, затем в базе. Недоступный Redis не ломает редиректы: через 300 мс
/// запрос идёт в базу. Счётчик переходов увеличивается фоновой задачей уже после ответа.
#[get("/r/{code}", tags = ["redirect"])]
async fn follow(
    db: Db,
    cache: Cache,
    tasks: BackgroundTasks,
    Path(code): Path<String>,
) -> Result<Redirect> {
    let key = cache_key(&code);
    let url = match best_effort("get", cache.get::<String>(&key))
        .await
        .flatten()
    {
        Some(url) => url,
        None => {
            let link = Link::select()
                .eq("code", code.as_str())
                .first(&db)
                .await?
                .ok_or_else(|| HTTPException::new(404, "Link not found"))?;
            best_effort("set", cache.set(&key, &link.url, CACHE_TTL)).await;
            link.url
        }
    };
    tasks.add(async move {
        rustyapi::sqlx::query("UPDATE links SET clicks = clicks + 1 WHERE code = $1")
            .bind(code)
            .execute(&*db)
            .await
    });
    Ok(Redirect::temporary(&url))
}

pub fn router() -> APIRouter {
    let links = APIRouter::new()
        .prefix("/links")
        .tags(["links"])
        .route(create_link)
        .route(list_links)
        .route(read_link)
        .route(delete_link);
    APIRouter::new().include_router(links).route(follow)
}
