//! Pure business logic of the friends feature, consumable from both REST
//! and WebSocket RPC endpoints. Functions take an authenticated user id
//! plus parameters and return typed results / [`ApiError`].
//!
//! No `unwrap`/`panic` — all fallible paths bubble up through `Result`.

use std::sync::Arc;

use uuid::Uuid;

use crate::errors::ApiError;
use crate::models::{
    FriendRequestResult, PendingRequest, RemovedResponse, StatusResponse, UserSummary,
};
use crate::gateway::broadcast::notify_user;
use crate::gateway::models::ServerMessage;
use crate::gateway::state::AppState;
use crate::store::FriendRecord;

#[inline]
fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Returns the accepted friends of `user_id`.
pub fn list_friends(state: &Arc<AppState>, user_id: &str) -> Vec<UserSummary> {
    let store = &state.auth_store;
    store
        .friends
        .iter()
        .filter(|r| {
            r.value().status == "accepted"
                && (r.value().from_user_id == user_id || r.value().to_user_id == user_id)
        })
        .filter_map(|r| {
            let other_id = if r.value().from_user_id == user_id {
                &r.value().to_user_id
            } else {
                &r.value().from_user_id
            };
            store
                .users
                .get(other_id)
                .map(|u| UserSummary::from(u.value()))
        })
        .collect()
}

/// Returns the pending requests *received* by `user_id`.
pub fn list_pending(state: &Arc<AppState>, user_id: &str) -> Vec<PendingRequest> {
    let store = &state.auth_store;
    store
        .friends
        .iter()
        .filter(|r| r.value().to_user_id == user_id && r.value().status == "pending")
        .filter_map(|r| {
            let f = r.value();
            store.users.get(&f.from_user_id).map(|u| PendingRequest {
                id: f.id.clone(),
                from: Some(UserSummary::from(u.value())),
                created_at_ms: f.created_at_ms,
            })
        })
        .collect()
}

/// Sends a friend request from `from_user_id` to `to_user_id`.
/// Pushes a `FriendRequestReceived` WS event to the recipient on success.
pub async fn send_request(
    state: &Arc<AppState>,
    from_user_id: String,
    to_user_id: String,
) -> Result<FriendRequestResult, ApiError> {
    if from_user_id == to_user_id {
        return Err(ApiError::BadRequest("Cannot add yourself".into()));
    }
    let store = &state.auth_store;
    if !store.users.contains_key(&to_user_id) {
        return Err(ApiError::NotFound("User not found".into()));
    }

    // Block on pending/accepted, clear stale rejects.
    let stale_id: Option<String> = {
        let _found = store.friends.iter().find(|r| {
            let f = r.value();
            (f.from_user_id == from_user_id && f.to_user_id == to_user_id)
                || (f.from_user_id == to_user_id && f.to_user_id == from_user_id)
        });
        match _found {
            None => None,
            Some(r) => match r.value().status.as_str() {
                "pending" | "accepted" => {
                    return Err(ApiError::Conflict("Friend request already exists".into()));
                }
                _ => Some(r.key().clone()),
            },
        }
    };
    if let Some(stale) = stale_id {
        store.friends.remove(&stale);
    }

    let id = Uuid::new_v4().to_string();
    let now_ms = epoch_ms();

    let sender_summary = store
        .users
        .get(&from_user_id)
        .map(|u| UserSummary::from(u.value()));

    store.friends.insert(
        id.clone(),
        FriendRecord {
            id: id.clone(),
            from_user_id,
            to_user_id: to_user_id.clone(),
            status: "pending".into(),
            created_at_ms: now_ms,
        },
    );
    store.mark_dirty();

    if let Some(from) = sender_summary {
        notify_user(
            state,
            &to_user_id,
            &ServerMessage::FriendRequestReceived {
                request: PendingRequest {
                    id: id.clone(),
                    from: Some(from),
                    created_at_ms: now_ms,
                },
            },
        )
        .await;
    }

    Ok(FriendRequestResult {
        id,
        status: "pending".into(),
    })
}

/// Accepts a pending friend request addressed to `user_id`.
pub async fn accept_request(
    state: &Arc<AppState>,
    user_id: &str,
    request_id: String,
) -> Result<StatusResponse, ApiError> {
    let store = &state.auth_store;
    let from_user_id = {
        let mut record = store
            .friends
            .get_mut(&request_id)
            .ok_or_else(|| ApiError::NotFound("Request not found".into()))?;
        if record.to_user_id != user_id || record.status != "pending" {
            return Err(ApiError::BadRequest("Cannot accept this request".into()));
        }
        record.status = "accepted".into();
        record.from_user_id.clone()
    };
    store.mark_dirty();

    let accepter_summary = store
        .users
        .get(user_id)
        .map(|u| UserSummary::from(u.value()));

    if let Some(friend) = accepter_summary {
        notify_user(
            state,
            &from_user_id,
            &ServerMessage::FriendRequestAccepted { request_id, friend },
        )
        .await;
    }

    Ok(StatusResponse {
        status: "accepted".into(),
    })
}

/// Rejects a pending friend request addressed to `user_id`.
pub async fn reject_request(
    state: &Arc<AppState>,
    user_id: &str,
    request_id: String,
) -> Result<StatusResponse, ApiError> {
    let store = &state.auth_store;
    let from_user_id = {
        let mut record = store
            .friends
            .get_mut(&request_id)
            .ok_or_else(|| ApiError::NotFound("Request not found".into()))?;
        if record.to_user_id != user_id || record.status != "pending" {
            return Err(ApiError::BadRequest("Cannot reject this request".into()));
        }
        record.status = "rejected".into();
        record.from_user_id.clone()
    };
    store.mark_dirty();

    notify_user(
        state,
        &from_user_id,
        &ServerMessage::FriendRequestDeclined {
            request_id,
            by_user_id: user_id.to_string(),
        },
    )
    .await;

    Ok(StatusResponse {
        status: "rejected".into(),
    })
}

/// Removes a friendship by id (cancel-pending or unfriend-accepted).
pub async fn remove_friendship(
    state: &Arc<AppState>,
    user_id: &str,
    friendship_id: String,
) -> Result<RemovedResponse, ApiError> {
    let store = &state.auth_store;
    let snapshot = {
        let record = store
            .friends
            .get(&friendship_id)
            .ok_or_else(|| ApiError::NotFound("Friendship not found".into()))?;
        if record.from_user_id != user_id && record.to_user_id != user_id {
            return Err(ApiError::BadRequest("Not your friendship".into()));
        }
        (
            record.from_user_id.clone(),
            record.to_user_id.clone(),
            record.status.clone(),
        )
    };
    store.friends.remove(&friendship_id);
    store.mark_dirty();

    let (from, to, status) = snapshot;
    let other = if from == user_id { to } else { from };
    push_removal_event(state, friendship_id, other, user_id.to_string(), &status).await;

    Ok(RemovedResponse { removed: true })
}

/// Removes a friendship identified by the *other* user's id.
pub async fn remove_friend_by_user(
    state: &Arc<AppState>,
    user_id: &str,
    target_user_id: String,
) -> Result<RemovedResponse, ApiError> {
    let store = &state.auth_store;
    let entry = {
        let _found = store.friends.iter().find(|r| {
            let f = r.value();
            (f.from_user_id == user_id && f.to_user_id == target_user_id)
                || (f.from_user_id == target_user_id && f.to_user_id == user_id)
        });
        _found.map(|r| (r.key().clone(), r.value().status.clone()))
    };
    let (id, status) = entry.ok_or_else(|| ApiError::NotFound("Friendship not found".into()))?;
    store.friends.remove(&id);
    store.mark_dirty();
    push_removal_event(state, id, target_user_id, user_id.to_string(), &status).await;
    Ok(RemovedResponse { removed: true })
}

async fn push_removal_event(
    state: &Arc<AppState>,
    friendship_id: String,
    other_user_id: String,
    actor_user_id: String,
    status: &str,
) {
    match status {
        "pending" => {
            notify_user(
                state,
                &other_user_id,
                &ServerMessage::FriendRequestCancelled {
                    request_id: friendship_id,
                    by_user_id: actor_user_id,
                },
            )
            .await;
        }
        "accepted" => {
            notify_user(
                state,
                &other_user_id,
                &ServerMessage::FriendRemoved {
                    friendship_id,
                    by_user_id: actor_user_id,
                },
            )
            .await;
        }
        _ => {}
    }
}
