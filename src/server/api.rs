use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::session::manager::SessionManager;
use crate::session::model::ToolKind;

type AppState = Arc<SessionManager>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/input", post(send_input))
        .route("/api/sessions/{id}/stop", post(stop_session))
        .route("/api/sessions/{id}/resize", post(resize_session))
        .route("/api/sessions/{id}/open-iterm", post(open_iterm))
}

async fn health() -> &'static str {
    "OK"
}

async fn list_sessions(State(mgr): State<AppState>) -> impl IntoResponse {
    let sessions = mgr.list().await;
    Json(sessions)
}

#[derive(Deserialize)]
struct CreateSessionRequest {
    name: Option<String>,
    tool: Option<String>,
    working_dir: Option<PathBuf>,
    extra_args: Option<Vec<String>>,
    auto_open_iterm: Option<bool>,
    rows: Option<u16>,
    cols: Option<u16>,
}

async fn create_session(
    State(mgr): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    let tool: ToolKind = req
        .tool
        .unwrap_or_else(|| mgr.config.default_tool.clone())
        .parse()
        .map_err(|e: String| (StatusCode::BAD_REQUEST, e))?;

    let name = req.name.unwrap_or_else(|| format!("{tool} session"));
    let working_dir = req
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let extra_args = req.extra_args.unwrap_or_default();

    let rows = req.rows.unwrap_or(24);
    let cols = req.cols.unwrap_or(80);

    match mgr
        .spawn(name, tool, working_dir.clone(), extra_args, rows, cols)
        .await
    {
        Ok(meta) => {
            // Optionally open in iTerm2
            if req.auto_open_iterm.unwrap_or(false)
                && mgr.config.iterm_enabled
                && let Err(e) = crate::iterm::open_in_iterm(meta.id, &working_dir)
            {
                tracing::warn!("Failed to open iTerm2: {e}");
            }
            Ok((StatusCode::CREATED, Json(meta)))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn get_session(State(mgr): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match mgr.get(id).await {
        Ok(meta) => Ok(Json(meta)),
        Err(_) => Err((StatusCode::NOT_FOUND, "Session not found")),
    }
}

#[derive(Deserialize)]
struct InputRequest {
    text: String,
}

async fn send_input(
    State(mgr): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<InputRequest>,
) -> impl IntoResponse {
    match mgr.send_input(id, req.text.into_bytes()).await {
        Ok(()) => Ok(StatusCode::OK),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(Deserialize)]
struct ResizeRequest {
    rows: u16,
    cols: u16,
}

async fn resize_session(
    State(mgr): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ResizeRequest>,
) -> impl IntoResponse {
    if req.rows == 0 || req.rows > 500 || req.cols == 0 || req.cols > 500 {
        return Err((
            StatusCode::BAD_REQUEST,
            "rows and cols must be 1-500".to_string(),
        ));
    }
    match mgr.resize(id, req.rows, req.cols).await {
        Ok(()) => Ok(StatusCode::OK),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

async fn stop_session(State(mgr): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match mgr.stop(id).await {
        Ok(()) => Ok(StatusCode::OK),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

async fn open_iterm(State(mgr): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    if !mgr.config.iterm_enabled {
        return Err((
            StatusCode::BAD_REQUEST,
            "iTerm2 integration disabled".to_string(),
        ));
    }

    let meta = mgr
        .get(id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    match crate::iterm::open_in_iterm(id, &meta.working_dir) {
        Ok(()) => Ok(StatusCode::OK),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
