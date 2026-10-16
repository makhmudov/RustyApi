//! WebSocket: `#[websocket("/ws")]` и типы axum.
//!
//! ```
//! use rustyapi::prelude::*;
//! use rustyapi::websocket::{Message, WebSocket};
//!
//! #[websocket("/ws/{room}")]
//! async fn chat(mut socket: WebSocket, Path(room): Path<String>) {
//!     while let Some(Ok(Message::Text(text))) = socket.recv().await {
//!         let reply = format!("{room}: {text}");
//!         if socket.send(Message::Text(reply.into())).await.is_err() {
//!             break;
//!         }
//!     }
//! }
//! ```

pub use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
