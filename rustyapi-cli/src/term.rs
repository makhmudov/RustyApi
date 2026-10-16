//! Цвета в терминале (только если stderr — терминал и не задан NO_COLOR).

use std::io::IsTerminal;
use std::sync::LazyLock;

static COLOR: LazyLock<bool> =
    LazyLock::new(|| std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none());

fn paint(code: &str, text: &str) -> String {
    if *COLOR {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

pub fn red(text: &str) -> String {
    paint("1;31", text)
}

pub fn green(text: &str) -> String {
    paint("1;32", text)
}

pub fn yellow(text: &str) -> String {
    paint("1;33", text)
}

pub fn cyan(text: &str) -> String {
    paint("1;36", text)
}

pub fn dim(text: &str) -> String {
    paint("2", text)
}

/// Строка статуса `tez`: `  tez  текст`.
pub fn status(text: &str) {
    eprintln!("{} {text}", cyan("  tez"));
}
