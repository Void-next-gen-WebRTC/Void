//! WS-RPC dispatcher. Maps `method` strings to typed handlers, all of which
//! require the WS to have completed an `Authenticate` step (auth_user_id
//! must be set). Handlers reuse the REST-side core logic verbatim.
//!
//! Result envelope: `ServerMessage::RpcResult { request_id, result?, error? }`.
//! No `unwrap`/`panic` — every fallible path returns a structured `RpcError`.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use super::broadcast::serialize_message;
use super::dm as ws_dm;
use super::models::{RpcError, ServerMessage};
use super::state::AppState;
use crate::errors::ApiError;
use crate::friends::core as friends_core;
use crate::metrics::WS_QUEUE_DROPPED;
use crate::models::UserSummary;

#[derive(Deserialize)]
struct ToUser {
    #[serde(rename = "toUserId", alias = "to_user_id")]
    to_user_id: String,
}

#[derive(Deserialize)]
struct ById {
    id: String,
}

#[derive(Deserialize)]
struct ByUserId {
    #[serde(rename = "userId", alias = "user_id")]
    user_id: String,
}

#[derive(Deserialize)]
struct ServerIdParam {
    #[serde(rename = "serverId", alias = "server_id")]
    server_id: String,
}

#[derive(Deserialize)]
struct ChannelIdParam {
    #[serde(rename = "channelId", alias = "channel_id")]
    channel_id: String,
}

/// Dispatches an RPC call. The reply (success or error) is delivered through
/// `tx` so the caller does not have to handle serialization.
pub async fn dispatch(
    state: &Arc<AppState>,
    auth_user_id: Option<&str>,
    request_id: String,
    method: String,
    params: Value,
    tx: &tokio::sync::mpsc::Sender<String>,
) {
    let Some(uid) = auth_user_id else {
        return reply_error(tx, request_id, "unauthorized", "WS not authenticated").await;
    };

    let outcome = run(state, uid, &method, params).await;
    match outcome {
        Ok(result) => reply_ok(tx, request_id, result).await,
        Err((code, message)) => reply_error(tx, request_id, code, &message).await,
    }
}

async fn run(
    state: &Arc<AppState>,
    user_id: &str,
    method: &str,
    params: Value,
) -> Result<Value, (&'static str, String)> {
    match method {
        // ---- Friends ----
        "friends.list" => {
            let items = friends_core::list_friends(state, user_id);
            Ok(json!(items))
        }
        "friends.pending" => {
            let items = friends_core::list_pending(state, user_id);
            Ok(json!(items))
        }
        "friends.send" => {
            let p: ToUser = decode(params)?;
            let r = friends_core::send_request(state, user_id.to_string(), p.to_user_id)
                .await
                .map_err(api_to_rpc)?;
            Ok(json!(r))
        }
        "friends.accept" => {
            let p: ById = decode(params)?;
            let r = friends_core::accept_request(state, user_id, p.id)
                .await
                .map_err(api_to_rpc)?;
            Ok(json!(r))
        }
        "friends.reject" => {
            let p: ById = decode(params)?;
            let r = friends_core::reject_request(state, user_id, p.id)
                .await
                .map_err(api_to_rpc)?;
            Ok(json!(r))
        }
        "friends.remove" => {
            let p: ById = decode(params)?;
            let r = friends_core::remove_friendship(state, user_id, p.id)
                .await
                .map_err(api_to_rpc)?;
            Ok(json!(r))
        }
        "friends.removeByUser" => {
            let p: ByUserId = decode(params)?;
            let r = friends_core::remove_friend_by_user(state, user_id, p.user_id)
                .await
                .map_err(api_to_rpc)?;
            Ok(json!(r))
        }

        // ---- Server members ----
        "server.members" => {
            let p: ServerIdParam = decode(params)?;
            let members = list_server_members(state, &p.server_id)
                .ok_or_else(|| ("not-found", "Server not found".to_string()))?;
            Ok(json!(members))
        }

        // ---- Chat history ----
        "chat.history" => {
            let p: ChannelIdParam = decode(params)?;
            let history = state.chat_history.read().await;
            let entries: Vec<_> = history
                .get(&p.channel_id)
                .map(|buf| buf.iter().cloned().collect())
                .unwrap_or_default();
            drop(history);
            Ok(json!(entries))
        }

        // ---- Direct messages ----
        "dm.history" => {
            let p: ByUserId = decode(params)?;
            let entries = ws_dm::dm_history(state, user_id, &p.user_id)
                .await
                .map_err(api_to_rpc)?;
            Ok(json!(entries))
        }
        "dm.partners" => {
            let partners = ws_dm::list_recent_dm_partners(state, user_id).await;
            Ok(json!(partners))
        }

        _ => Err(("unknown-method", format!("Unknown method: {}", method))),
    }
}

fn list_server_members(state: &AppState, server_id: &str) -> Option<Vec<UserSummary>> {
    let server = state.server_registry.servers.get(server_id)?;
    let members = server
        .members
        .iter()
        .filter_map(|pk| {
            let uid = state.auth_store.pubkey_index.get(pk)?;
            let record = state.auth_store.users.get(uid.value())?;
            Some(UserSummary::from(record.value()))
        })
        .collect();
    Some(members)
}

fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, (&'static str, String)> {
    serde_json::from_value(v).map_err(|e| ("bad-params", e.to_string()))
}

fn api_to_rpc(e: ApiError) -> (&'static str, String) {
    match e {
        ApiError::BadRequest(m) => ("bad-request", m),
        ApiError::NotFound(m) => ("not-found", m),
        ApiError::Conflict(m) => ("conflict", m),
        ApiError::Forbidden(m) => ("forbidden", m),
        ApiError::Unauthorized(m) => ("unauthorized", m),
        other => ("internal", format!("{:?}", other)),
    }
}

async fn reply_ok(tx: &tokio::sync::mpsc::Sender<String>, request_id: String, result: Value) {
    let msg = ServerMessage::RpcResult {
        request_id,
        result: Some(result),
        error: None,
    };
    if let Some(payload) = serialize_message(&msg) {
        if tx.try_send(payload).is_err() {
            WS_QUEUE_DROPPED.inc();
        }
    }
}

async fn reply_error(
    tx: &tokio::sync::mpsc::Sender<String>,
    request_id: String,
    code: &str,
    message: &str,
) {
    debug!("rpc error [{}]: {}", code, message);
    let msg = ServerMessage::RpcResult {
        request_id,
        result: None,
        error: Some(RpcError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    };
    if let Some(payload) = serialize_message(&msg) {
        if tx.try_send(payload).is_err() {
            WS_QUEUE_DROPPED.inc();
        }
    }
}
