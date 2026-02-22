use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::session::manager::SessionManager;

type AppState = Arc<SessionManager>;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/sessions/{id}/logs", get(stream_logs))
}

async fn stream_logs(State(mgr): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let snapshot = match mgr.get_log_snapshot(id).await {
        Ok(s) => s,
        Err(_) => return Err((StatusCode::NOT_FOUND, "Session not found")),
    };

    let log_rx = match mgr.subscribe_logs(id).await {
        Ok(r) => r,
        Err(_) => return Err((StatusCode::NOT_FOUND, "Session not found")),
    };

    let mut size_rx = match mgr.subscribe_size(id).await {
        Ok(r) => r,
        Err(_) => return Err((StatusCode::NOT_FOUND, "Session not found")),
    };

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(256);

    tokio::spawn(async move {
        // Send initial resize event with current PTY size
        let (rows, cols) = *size_rx.borrow_and_update();
        let resize_data = serde_json::json!({ "rows": rows, "cols": cols });
        let _ = tx
            .send(Ok(Event::default()
                .event("resize")
                .data(resize_data.to_string())))
            .await;

        // Send log snapshot
        for entry in snapshot {
            let _ = tx
                .send(Ok(Event::default()
                    .event("log")
                    .data(serde_json::to_string(&entry).unwrap_or_default())))
                .await;
        }

        // Merge live log + resize events
        let mut log_stream = tokio_stream::wrappers::BroadcastStream::new(log_rx);

        loop {
            tokio::select! {
                Some(result) = log_stream.next() => {
                    let event = match result {
                        Ok(entry) => Event::default()
                            .event("log")
                            .data(serde_json::to_string(&entry).unwrap_or_default()),
                        Err(_lagged) => Event::default()
                            .event("gap")
                            .data("Missed messages, refresh for full log"),
                    };
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
                Ok(()) = size_rx.changed() => {
                    let (rows, cols) = *size_rx.borrow_and_update();
                    let data = serde_json::json!({ "rows": rows, "cols": cols });
                    let event = Event::default()
                        .event("resize")
                        .data(data.to_string());
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
