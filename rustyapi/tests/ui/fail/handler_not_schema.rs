use rustyapi::prelude::*;

#[derive(rustyapi::serde::Deserialize)]
#[serde(crate = "rustyapi::serde")]
struct Item {
    name: String,
}

#[post("/items")]
async fn create_item(Json(item): Json<Item>) -> String {
    item.name
}

fn main() {}
