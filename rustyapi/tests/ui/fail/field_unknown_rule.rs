use rustyapi::prelude::*;

#[schema]
struct Item {
    #[field(min = 1)]
    price: i64,
}

fn main() {}
