use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use uuid::Uuid;

use crate::session::manager::SessionManager;
use crate::session::model::SessionMeta;

type AppState = Arc<SessionManager>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index_page))
        .route("/sessions/{id}", get(session_page))
        .route("/new", get(new_page))
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    sessions: Vec<SessionMeta>,
}

async fn index_page(State(mgr): State<AppState>) -> impl IntoResponse {
    let sessions = mgr.list().await;
    let template = IndexTemplate { sessions };
    HtmlTemplate(template)
}

#[derive(Template)]
#[template(path = "session.html")]
struct SessionTemplate {
    session: SessionMeta,
}

async fn session_page(State(mgr): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match mgr.get(id).await {
        Ok(session) => {
            let template = SessionTemplate { session };
            Ok(HtmlTemplate(template))
        }
        Err(_) => Err((StatusCode::NOT_FOUND, "Session not found")),
    }
}

#[derive(Template)]
#[template(path = "new.html")]
struct NewTemplate;

async fn new_page() -> impl IntoResponse {
    HtmlTemplate(NewTemplate)
}

struct HtmlTemplate<T>(T);

impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> axum::response::Response {
        match self.0.render() {
            Ok(html) => axum::response::Html(html).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Template error: {e}"),
            )
                .into_response(),
        }
    }
}
