use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::Router;

use crate::auth::middleware::AuthUser;
use crate::errors::ApiError;
use crate::gateway::state::AppState;
use crate::models::*;
use crate::negotiate::{accepts_proto, decode_body, negotiate, negotiate_list, Negotiated};

pub mod core;

/// Builds the `/api/friends` sub-router.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_friends))
        .route("/request", post(send_request))
        .route("/pending", get(list_pending))
        .route("/:id/accept", post(accept_request))
        .route("/:id/reject", post(reject_request))
        .route("/:id", delete(remove_friend))
        .route("/by-user/:user_id", delete(remove_friend_by_user))
}

async fn list_friends(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let items = core::list_friends(&state, &auth.user_id);
    Ok(negotiate_list(
        items,
        |items| UserSummaryList { items },
        proto,
    ))
}

async fn send_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let body: FriendRequestBody = decode_body(&headers, &body)?;
    let result = core::send_request(&state, auth.user_id, body.to_user_id).await?;
    Ok(negotiate(result, proto))
}

async fn list_pending(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let items = core::list_pending(&state, &auth.user_id);
    Ok(negotiate_list(
        items,
        |items| PendingRequestList { items },
        proto,
    ))
}

async fn accept_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let result = core::accept_request(&state, &auth.user_id, id).await?;
    Ok(negotiate(result, proto))
}

async fn reject_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let result = core::reject_request(&state, &auth.user_id, id).await?;
    Ok(negotiate(result, proto))
}

async fn remove_friend(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let result = core::remove_friendship(&state, &auth.user_id, id).await?;
    Ok(negotiate(result, proto))
}

async fn remove_friend_by_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(target_user_id): Path<String>,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let result = core::remove_friend_by_user(&state, &auth.user_id, target_user_id).await?;
    Ok(negotiate(result, proto))
}
