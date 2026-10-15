# FastAPI → RustyAPI

FastAPI'ni biladiganlar uchun moslik jadvali. Javoblar (422, 401, 404, OpenAPI) FastAPI bilan bir
xil: testlar ularni haqiqiy FastAPI'dan olingan namunalar bilan solishtiradi.

## Ilova va marshrutlar

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

## Ma'lumotlar va validatsiya

| FastAPI / Pydantic | RustyAPI |
|---|---|
| `class Item(BaseModel)` | `#[schema] struct Item` |
| `name: str = Field(min_length=1, max_length=100)` | `#[field(min_length = 1, max_length = 100)] name: String` |
| `price: float = Field(gt=0)` | `#[field(gt = 0)] price: f64` |
| `ge`, `lt`, `le`, `pattern` | nomlari bir xil |
| `email: EmailStr` | `#[field(email)] email: String` |
| `url: HttpUrl` | `#[field(url)] url: String` |
| `quantity: int = 1` | `#[field(default = 1)] quantity: i64` |
| `description: str \| None = None` | `description: Option<String>` |
| `tags: list[str] = []` | `#[serde(default)] tags: Vec<String>` |
| `class Status(str, Enum)` | `#[schema] #[serde(rename_all = "lowercase")] enum Status { Todo, Done }` |
| `Field(alias="userName")` | `#[serde(rename = "userName")]` |
| so'rov tanasi `item: Item` | `Json(item): Json<Item>` |
| query parametrlar `q: str, limit: int = 10` | `Query(params): Query<Search>` (`#[schema] struct Search`) |
| `Form(...)` | `Form(data): Form<Login>` |
| `UploadFile`, `File()` | `file: UploadFile`, `form: MultipartForm` |
| `response_model=Item` | qaytish turi `Json<Item>` |

## Bog'liqliklar va xavfsizlik

| FastAPI | RustyAPI |
|---|---|
| `db: Session = Depends(get_db)` | `db: Db` |
| `user = Depends(get_current_user)` | `Depends(user): Depends<CurrentUser>` + `impl Dependency for CurrentUser` |
| `OAuth2PasswordBearer(tokenUrl="token")` | `auth: Auth` ekstraktori |
| `OAuth2PasswordBearer(..., auto_error=False)` | `auth: Option<Auth>` |
| `form: OAuth2PasswordRequestForm = Depends()` | `form: OAuth2PasswordRequestForm` |
| `pwd_context.hash` / `verify` | `auth::hash_password` / `auth::verify_password` |
| `jwt.encode({"sub": ...})` | `auth::create_access_token(user.id)` |
| `Security(..., scopes=["admin"])` | `auth.require_role("admin")?` |

## Qolganlari

| FastAPI | RustyAPI |
|---|---|
| SQLAlchemy modeli | `#[model(table = "items")] struct Item` |
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
| `/docs`, `/redoc`, `/openapi.json` | manzillar bir xil |
| `openapi_url=None` | `.no_openapi()` |

## Nimasi bilan farq qiladi

- Handlerlar faqat `async fn`; argumentlar aniq turli ekstraktorlar (`Path`, `Query`, `Json`).
- Turlar Pydantic'ning lax rejimidan qat'iyroq: `true` `1.0` ga aylanmaydi, JSON'dagi `"5"` satri
  son hisoblanmaydi. Query va path satrlari FastAPI'dagidek tahlil qilinadi.
- Tur yoki `#[field]` qoidasidagi xato ishga tushirishda emas, kompilyatsiyada chiqadi.
- Xato `raise` bilan emas, `Err(...)` va `?` bilan qaytariladi.
