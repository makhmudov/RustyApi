//! Настоящий сервер: TCP, access-лог, корректная остановка.

use rustyapi::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[get("/")]
async fn root() -> Json<Value> {
    Json(json!({ "message": "Hello" }))
}

#[tokio::test]
async fn serves_and_shuts_down_gracefully() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(App::new().route(root).serve_with_shutdown(listener, async {
        let _ = stopped.await;
    }));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /missing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 404 Not Found"), "{response}");
    assert!(
        response.ends_with(r#"{"detail":"Not Found"}"#),
        "{response}"
    );

    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
