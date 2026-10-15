use rustyapi::prelude::*;

#[get("/items/{item_id:int}")]
async fn read_item() {}

fn main() {}
