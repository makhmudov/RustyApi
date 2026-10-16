# FastAPI → RustyAPI

Таблица соответствий для тех, кто знает FastAPI. Ответы (422, 401, 404, OpenAPI) совпадают с
FastAPI; это проверяется тестами на эталонах, снятых с настоящего FastAPI.

## Приложение и роуты

| FastAPI | RustyAPI |
|---|---|
| `app = FastAPI(title="Shop")` | `App::new().title("Shop")` |
| `@app.get("/items/{item_id}")` | `#[get("/items/{item_id}")]` + `.route(read_item)` |
| `def read_item(item_id: int)` | `async fn read_item(Path(item_id): Path<i64>)` |
| `@app.post("/items", status_code=201)` | `#[post("/items", status_code = 201)]` |
| `tags=["items"], summary=..., deprecated=True` | `tags = ["items"], summary = "...", deprecated` |
| `router = APIRouter(prefix="/items", tags=["items"])` | `APIRouter::new().prefix("/items").tags(["items"])` |
| `app.include_router(router)` | `.include_router(router)` |
| `raise HTTPException(404, "Item not found")` | `return Err(HTTPException::new(404, "Item not found").into())` |
| `uvicorn main:app --reload` | `tez dev` |

## Данные и валидация

| FastAPI / Pydantic | RustyAPI |
|---|---|
| `class Item(BaseModel)` | `#[schema] struct Item` |
| `name: str = Field(min_length=1, max_length=100)` | `#[field(min_length = 1, max_length = 100)] name: String` |
| `price: float = Field(gt=0)` | `#[field(gt = 0)] price: f64` |
| `ge`, `lt`, `le`, `pattern` | те же имена |
| `email: EmailStr` | `#[field(email)] email: String` |
| `url: HttpUrl` | `#[field(url)] url: String` |
| `quantity: int = 1` | `#[field(default = 1)] quantity: i64` |
| `description: str \| None = None` | `description: Option<String>` |
| `tags: list[str] = []` | `#[serde(default)] tags: Vec<String>` |
| `class Status(str, Enum)` | `#[schema] #[serde(rename_all = "lowercase")] enum Status { Todo, Done }` |
| `Field(alias="userName")` | `#[serde(rename = "userName")]` |
| тело `item: Item` | `Json(item): Json<Item>` |
| query-параметры `q: str, limit: int = 10` | `Query(params): Query<Search>` (`#[schema] struct Search`) |
| `Form(...)` | `Form(data): Form<Login>` |
| `UploadFile`, `File()` | `file: UploadFile`, `form: MultipartForm` |
| `response_model=Item` | возвращаемый тип `Json<Item>` |

## Зависимости и безопасность

| FastAPI | RustyAPI |
|---|---|
| `db: Session = Depends(get_db)` | `db: Db` |
| `user = Depends(get_current_user)` | `Depends(user): Depends<CurrentUser>` + `impl Dependency for CurrentUser` |
| `OAuth2PasswordBearer(tokenUrl="token")` | экстрактор `auth: Auth` |
| `OAuth2PasswordBearer(..., auto_error=False)` | `auth: Option<Auth>` |
| `form: OAuth2PasswordRequestForm = Depends()` | `form: OAuth2PasswordRequestForm` |
| `pwd_context.hash` / `verify` | `auth::hash_password` / `auth::verify_password` |
| `jwt.encode({"sub": ...})` | `auth::create_access_token(user.id)` |
| `Security(..., scopes=["admin"])` | `auth.require_role("admin")?` |

## Остальное

| FastAPI | RustyAPI |
|---|---|
| SQLAlchemy-модель | `#[model(table = "items")] struct Item` |
| `session.get(Item, id)` / 404 | `Item::get_or_404(&db, id).await?` |
| `select(Item).where(Item.done == True)` | `Item::select().eq("done", true).all(&db).await?` |
| Alembic | `tez migrate new/run/revert` |
| `BackgroundTasks` | `tasks: BackgroundTasks`, `tasks.add(async { ... })` |
| `@app.websocket("/ws")` | `#[websocket("/ws")] async fn ws(socket: WebSocket)` |
| `CORSMiddleware` | `.cors_origins(["https://app.example.com"])` |
| `GZipMiddleware` | `.gzip()` |
| `app.mount("/static", StaticFiles(directory="static"))` | `.mount("/static", "static")` |
| `BaseSettings` | `#[schema] struct Settings` + `rustyapi::settings()?` |
| `TestClient(app)` | `TestClient::new(app())` |
| `/docs`, `/redoc`, `/openapi.json` | те же адреса |
| `openapi_url=None` | `.no_openapi()` |

## Чем отличается

- Обработчики только `async fn`, аргументы — экстракторы с явными типами (`Path`, `Query`, `Json`).
- Типы строже Pydantic в lax-режиме: `true` не превращается в `1.0`, строка `"5"` в JSON не становится
  числом. Строки query и path разбираются, как в FastAPI.
- Ошибка в типах или правилах `#[field]` — ошибка компиляции, а не исключение при запуске.
- Возврат ошибки — `Err(...)` и `?`, а не `raise`.
