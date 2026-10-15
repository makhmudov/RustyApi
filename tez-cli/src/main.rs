//! CLI `tez`. Команды new / dev / run / migrate / generate / routes появятся на этапе 6.

use clap::Parser;

/// Инструмент разработчика для фреймворка Tez.
#[derive(Debug, Parser)]
#[command(name = "tez", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!(
        "tez {}: команды появятся на этапе 6",
        env!("CARGO_PKG_VERSION")
    );
}
