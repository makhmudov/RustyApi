# FastAPI → RustyAPI

A mapping for people who know FastAPI. Responses (422, 401, 404, OpenAPI) match FastAPI; tests
compare them with fixtures recorded from real FastAPI.

## App and routes

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

## Data and validation

| FastAPI / Pydantic | RustyAPI |
|---|---|
| `class Item(BaseModel)` | `#[schema] struct Item` |
| `name: str = Field(min_length=1, max_length=100)` | `#[field(min_length = 1, max_length = 100)] name: String` |
| `price: float = Field(gt=0)` | `#[field(gt = 0)] price: f64` |
| `ge`, `lt`, `le`, `pattern` | same names |
| `email: EmailStr` | `#[field(email)] email: String` |
| `url: HttpUrl` | `#[field(url)] url: String` |
| `quantity: int = 1` | `#[field(default = 1)] quantity: i64` |
| `description: str \| None = None` | `description: Option<String>` |
| `tags: list[str] = []` | `#[serde(default)] tags: Vec<String>` |
| `class Status(str, Enum)` | `#[schema] #[serde(rename_all = "lowercase")] enum Status { Todo, Done }` |
| `Field(alias="userName")` | `#[serde(rename = "userName")]` |
| body `item: Item` | `Json(item): Json<Item>` |
| query params `q: str, limit: int = 10` | `Query(params): Query<Search>` (`#[schema] struct Search`) |
| `Form(...)` | `Form(data): Form<Login>` |
| `UploadFile`, `File()` | `file: UploadFile`, `form: MultipartForm` |
| `response_model=Item` | return type `Json<Item>` |

## Dependencies and security

| FastAPI | RustyAPI |
|---|---|
| `db: Session = Depends(get_db)` | `db: Db` |
| `user = Depends(get_current_user)` | `Depends(user): Depends<CurrentUser>` + `impl Dependency for CurrentUser` |
| `OAuth2PasswordBearer(tokenUrl="token")` | the `auth: Auth` extractor |
| `OAuth2PasswordBearer(..., auto_error=False)` | `auth: Option<Auth>` |
| `form: OAuth2PasswordRequestForm = Depends()` | `form: OAuth2PasswordRequestForm` |
| `pwd_context.hash` / `verify` | `auth::hash_password` / `auth::verify_password` |
| `jwt.encode({"sub": ...})` | `auth::create_access_token(user.id)` |
| `Security(..., scopes=["admin"])` | `auth.require_role("admin")?` |

## Everything else

| FastAPI | RustyAPI |
|---|---|
| SQLAlchemy model | `#[model(table = "items")] struct Item` |
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
| `/docs`, `/redoc`, `/openapi.json` | same URLs |
| `openapi_url=None` | `.no_openapi()` |

## What is different

- Handlers are `async fn` only; arguments are typed extractors (`Path`, `Query`, `Json`).
- Types are stricter than Pydantic's lax mode: `true` does not become `1.0`, and the JSON string
  `"5"` is not a number. Query and path strings are parsed like FastAPI.
- A wrong type or `#[field]` rule is a compile error, not an exception at startup.
- Errors are returned with `Err(...)` and `?` instead of `raise`.
