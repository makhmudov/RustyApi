use rustyapi::prelude::*;

#[schema]
enum Shape {
    Circle { radius: f64 },
    Square,
}

fn main() {}
