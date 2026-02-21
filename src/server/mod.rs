pub mod api;
pub mod sse;
pub mod templates;

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use rust_embed::Embed;
use tower_http::cors::CorsLayer;

use crate::config::{Config, resolve_bind_address};
use crate::session::manager::SessionManager;

#[derive(Embed)]
#[folder = "static/"]
struct StaticAssets;

async fn serve_static(Path(path): Path<String>) -> impl IntoResponse {
    match StaticAssets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn start(config: Config) -> Result<()> {
    let bind = resolve_bind_address(&config.bind);
    let addr = format!("{bind}:{}", config.port);
    let manager = SessionManager::new(config.clone());

    // Restore sessions from disk
    restore_sessions(&manager).await;

    let state = Arc::new(manager);

    let app = Router::new()
        // API routes
        .merge(api::routes())
        // SSE routes
        .merge(sse::routes())
        // Template/page routes
        .merge(templates::routes())
        // Static files (embedded in binary)
        .route("/static/{*path}", axum::routing::get(serve_static))
        // CORS: deny all cross-origin requests (same-origin passes through)
        .layer(CorsLayer::new())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Lineforge v{} listening on http://{addr}", env!("CARGO_PKG_VERSION"));

    axum::serve(listener, app).await?;
    Ok(())
}

async fn restore_sessions(_manager: &SessionManager) {
    let sessions_dir = Config::sessions_dir();
    if !sessions_dir.exists() {
        return;
    }

    let entries = match std::fs::read_dir(&sessions_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let meta_path = entry.path().join("meta.json");
        if !meta_path.exists() {
            continue;
        }

        match std::fs::read_to_string(&meta_path) {
            Ok(content) => {
                match serde_json::from_str::<crate::session::model::SessionMeta>(&content) {
                    Ok(mut meta) => {
                        // Mark previously running sessions as stopped (they died with the server)
                        if meta.status == crate::session::model::SessionStatus::Running {
                            meta.status = crate::session::model::SessionStatus::Stopped;
                            meta.pid = None;
                            meta.updated_at = chrono::Utc::now();
                            if let Ok(json) = serde_json::to_string_pretty(&meta) {
                                let _ = std::fs::write(&meta_path, json);
                            }
                        }
                        tracing::debug!("Found previous session: {} ({})", meta.id, meta.name);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse session meta at {}: {e}",
                            meta_path.display()
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to read session meta at {}: {e}",
                    meta_path.display()
                );
            }
        }
    }
}
