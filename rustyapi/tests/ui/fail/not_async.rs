use rustyapi::prelude::*;

#[get("/")]
fn root() -> &'static str {
    "hello"
}

fn main() {}
