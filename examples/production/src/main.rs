//! Точка входа: настройки → приложение → сервер. Корректно завершается по SIGTERM (docker stop).
//!
//! `shortener healthcheck` — проверка для `HEALTHCHECK` в Docker: запрос к своему `/health`,
//! код выхода 0 при ответе 200. Так в образе не нужен curl.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn healthcheck() -> std::process::ExitCode {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".to_owned());
    let ok = (|| -> std::io::Result<bool> {
        let mut stream = TcpStream::connect(("127.0.0.1", port.parse().unwrap_or(8000)))?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
        let mut head = [0u8; 12];
        stream.read_exact(&mut head)?;
        Ok(head.ends_with(b" 200"))
    })();
    match ok {
        Ok(true) => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::FAILURE,
    }
}

#[rustyapi::main]
async fn main() -> rustyapi::Result<std::process::ExitCode> {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        return Ok(healthcheck());
    }
    let settings: shortener::Settings = rustyapi::settings()?;
    shortener::app(&settings).await?.run().await?;
    Ok(std::process::ExitCode::SUCCESS)
}
