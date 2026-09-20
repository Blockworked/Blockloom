//! The browser dev bridge: runs the real backend behind an HTTP + WebSocket
//! server so the frontend can be iterated on with `pnpm run dev` instead of
//! relaunching the CEF window after every UI tweak.
//!
//! `POST /invoke/<cmd>` runs a command; `GET /events` streams the state
//! snapshot on connect and after every change. Binds to 127.0.0.1 only, and is
//! feature-gated so it never ships in the editor binary.
//!
//! ```text
//! cargo run -p blockloom-app --features dev-bridge --bin blockloom-devserver
//! cd ui && pnpm run dev     # then open http://localhost:1420
//! ```

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use blockloom_app::{AppHandle, Backend, Event};
use serde_json::Value;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

const PORT: u16 = 4128;

#[derive(Clone)]
struct Shared {
    backend: Backend,
    events: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let (events, _) = broadcast::channel::<String>(64);
    let sink = events.clone();
    let backend = Backend::start(AppHandle::new(move |event| {
        if let Event::State(json) = event {
            // No subscribers yet is fine: the next tab to connect asks for a
            // fresh snapshot anyway.
            let _ = sink.send(json.to_string());
        }
    }));

    let shared = Shared { backend, events };
    let router = Router::new()
        .route("/invoke/{cmd}", post(invoke))
        .route("/events", get(subscribe))
        .layer(CorsLayer::permissive())
        .with_state(shared);

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!("dev bridge: couldn't bind 127.0.0.1:{PORT}: {e}");
            return;
        }
    };
    tracing::info!("dev bridge: listening on http://127.0.0.1:{PORT}");
    if let Err(e) = axum::serve(listener, router).await {
        tracing::error!("dev bridge: {e}");
    }
}

async fn invoke(
    Path(cmd): Path<String>,
    State(shared): State<Shared>,
    Json(args): Json<Value>,
) -> impl IntoResponse {
    match shared.backend.dispatch(&cmd, args) {
        Ok(data) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "data": data})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": e})),
        ),
    }
}

async fn subscribe(ws: WebSocketUpgrade, State(shared): State<Shared>) -> Response {
    ws.on_upgrade(move |socket| stream_events(socket, shared))
}

async fn stream_events(mut socket: WebSocket, shared: Shared) {
    let mut events = shared.events.subscribe();
    // A snapshot on connect, so a freshly opened tab has state immediately.
    if let Ok(initial) = shared.backend.state_json()
        && socket.send(Message::Text(initial.into())).await.is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => {
                let payload = match event {
                    Ok(json) => json,
                    // Lagged behind: send the current truth instead.
                    Err(broadcast::error::RecvError::Lagged(_)) => match shared.backend.state_json() {
                        Ok(json) => json,
                        Err(_) => continue,
                    },
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                if socket.send(Message::Text(payload.into())).await.is_err() {
                    return;
                }
            }
            // Nothing is expected from the client; this only notices it leaving.
            incoming = socket.recv() => {
                if incoming.is_none() {
                    return;
                }
            }
        }
    }
}
