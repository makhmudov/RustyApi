use rustyapi::prelude::*;

#[schema]
struct Item {
    #[field(pattern = "[a-z")]
    code: String,
}

fn main() {}
