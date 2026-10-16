use rustyapi::prelude::*;

#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({"message": "Hello"}))
}

#[rustyapi::main]
async fn main() -> rustyapi::Result<()> {
    // Не запускаем сервер (`.run()`), trybuild исполняет этот файл.
    let app = App::new().title("My API").route(root);
    assert_eq!(app.routes().len(), 1);
    let port: u16 = "8000".parse()?;
    assert_eq!(port, 8000);
    Ok(())
}
