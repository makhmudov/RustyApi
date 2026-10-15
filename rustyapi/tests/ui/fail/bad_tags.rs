use rustyapi::prelude::*;

#[get("/items", tags = [items])]
async fn read_items() {}

fn main() {}
