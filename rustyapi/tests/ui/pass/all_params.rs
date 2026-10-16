use rustyapi::prelude::*;

/// Документация попадает в description.
#[get("/items/{item_id}", status_code = 200, tags = ["items", "read"], summary = "Read Item", deprecated,)]
pub async fn read_item(Path(item_id): Path<u32>) -> String {
    item_id.to_string()
}

#[post("", status_code = status::HTTP_201_CREATED, tags = "items", description = "Создаёт")]
async fn create_item(Json(body): Json<Value>) -> Json<Value> {
    Json(body)
}

#[put("/items/{item_id}", deprecated = false)]
async fn replace_item(Path(_id): Path<u32>) -> StatusCode {
    StatusCode::OK
}

#[patch("/items/{item_id}", status_code = StatusCode::ACCEPTED)]
#[allow(unused_variables)]
async fn update_item(Path(id): Path<u32>) {}

#[delete("/items/{item_id}", status_code = 204)]
async fn delete_item() -> Result<()> {
    Ok(())
}

#[head("/files/{file_path:path}")]
async fn head_file() {}

#[options("/")]
async fn options_root() {}

mod nested {
    use rustyapi::prelude::*;

    #[get("/nested")]
    pub(crate) async fn nested() -> &'static str {
        "nested"
    }
}

#[rustyapi::main]
async fn main() {
    let items = APIRouter::new()
        .prefix("/items")
        .route(create_item)
        .route(read_item);
    let app = App::new()
        .route(replace_item)
        .route(update_item)
        .route(delete_item)
        .route(head_file)
        .route(options_root)
        .route(nested::nested)
        .include_router(items);
    let routes = app.routes();
    assert_eq!(routes.len(), 8);
    let create = routes.iter().find(|r| r.name == "create_item").unwrap();
    assert_eq!(create.path, "/items");
    assert_eq!(create.status_code, 201);
    assert_eq!(create.description.as_deref(), Some("Создаёт"));
    let _router = app.into_router();
}
