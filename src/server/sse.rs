use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::session::manager::SessionManager;

type AppState = Arc<SessionManager>;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/sessions/{id}/logs", get(stream_logs))
}

async fn stream_logs(State(mgr): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    // First send existing log entries, then stream new ones
    let snapshot = match mgr.get_log_snapshot(id).await {
        Ok(s) => s,
        Err(_) => return Err((StatusCode::NOT_FOUND, "Session not found")),
    };

    let rx = match mgr.subscribe_logs(id).await {
        Ok(r) => r,
        Err(_) => return Err((StatusCode::NOT_FOUND, "Session not found")),
    };

    let snapshot_stream = tokio_stream::iter(snapshot.into_iter().map(|entry| {
        Ok::<_, Infallible>(
            Event::default()
                .event("log")
                .data(serde_json::to_string(&entry).unwrap_or_default()),
        )
    }));

    let live_stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(entry) => Some(Ok::<_, Infallible>(
            Event::default()
                .event("log")
                .data(serde_json::to_string(&entry).unwrap_or_default()),
        )),
        Err(_lagged) => Some(Ok(Event::default()
            .event("gap")
            .data("Missed messages, refresh for full log"))),
    });

    let stream = snapshot_stream.chain(live_stream);

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
