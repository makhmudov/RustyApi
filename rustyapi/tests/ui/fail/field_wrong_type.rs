use rustyapi::prelude::*;

#[schema]
struct Item {
    #[field(gt = 0)]
    name: String,
}

fn main() {}
