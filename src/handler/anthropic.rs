use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;

use crate::server::AppState;

pub async fn handle(
    State(_state): State<Arc<AppState>>,
    _body: String,
) -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
