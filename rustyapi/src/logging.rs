//! Логи в стиле uvicorn:
//!
//! ```text
//! INFO:     Started server process [4242]
//! INFO:     RustyAPI running on http://127.0.0.1:8000 (Press CTRL+C to quit)
//! INFO:     127.0.0.1:51234 - "GET /items/1 HTTP/1.1" 200 OK
//! ```
//!
//! Уровень задаётся через `RUST_LOG` (по умолчанию `info`). Цвета включаются, только если stderr — терминал
//! и не задан `NO_COLOR`.

use std::fmt;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Цель (target) access-логов.
pub const ACCESS_TARGET: &str = "rustyapi::access";

static ANSI: AtomicBool = AtomicBool::new(false);

/// Устанавливает глобальный логгер в стиле uvicorn. Если логгер уже установлен (например,
/// пользователем), ничего не делает. Вызывается автоматически из `App::run`.
pub fn init() {
    let ansi = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let installed = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(ansi)
        .event_format(UvicornFormat)
        .try_init()
        .is_ok();
    if installed {
        ANSI.store(ansi, Ordering::Relaxed);
    }
}

pub(crate) fn ansi() -> bool {
    ANSI.load(Ordering::Relaxed)
}

/// Формат строки: `LEVEL:` дополненный пробелами до 9 символов, затем сообщение (как `%(levelprefix)s`).
struct UvicornFormat;

impl<S, N> FormatEvent<S, N> for UvicornFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let level = *event.metadata().level();
        let (name, color) = match level {
            Level::ERROR => ("ERROR", "31"),
            Level::WARN => ("WARNING", "33"),
            Level::INFO => ("INFO", "32"),
            Level::DEBUG => ("DEBUG", "36"),
            Level::TRACE => ("TRACE", "34"),
        };
        let pad = " ".repeat(8usize.saturating_sub(name.len()));
        if writer.has_ansi_escapes() {
            write!(writer, "\x1b[{color}m{name}\x1b[0m:{pad} ")?;
        } else {
            write!(writer, "{name}:{pad} ")?;
        }
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Строка access-лога: `127.0.0.1:51234 - "GET /items/1 HTTP/1.1" 200 OK`.
pub(crate) fn access_line(
    client: Option<std::net::SocketAddr>,
    request_line: &str,
    status: axum::http::StatusCode,
) -> String {
    let client = client.map_or_else(|| "-".to_owned(), |c| c.to_string());
    let status_text = match status.canonical_reason() {
        Some(reason) => format!("{} {reason}", status.as_u16()),
        None => status.as_u16().to_string(),
    };
    if ansi() {
        let color = match status.as_u16() {
            100..=199 => "97",
            200..=299 => "32",
            300..=399 => "33",
            400..=499 => "31",
            _ => "91",
        };
        format!("{client} - \"\x1b[1m{request_line}\x1b[0m\" \x1b[{color}m{status_text}\x1b[0m")
    } else {
        format!("{client} - \"{request_line}\" {status_text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_line_plain() {
        let addr = "127.0.0.1:5000".parse().ok();
        let line = access_line(addr, "GET / HTTP/1.1", axum::http::StatusCode::NOT_FOUND);
        assert_eq!(line, "127.0.0.1:5000 - \"GET / HTTP/1.1\" 404 Not Found");
    }
}
