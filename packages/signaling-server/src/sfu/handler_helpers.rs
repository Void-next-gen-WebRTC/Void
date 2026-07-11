// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! WebSocket auth + DM handlers, extracted from `handler.rs` to keep file
//! sizes within the 350-line ceiling. These functions are pure
//! transitions between an inbound [`super::models::ClientMessage`] and
//! either a state mutation or an outbound [`super::models::ServerMessage`].

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::debug;

use super::broadcast::{notify_user, serialize_message};
use super::dm;
use super::models::{RpcError, ServerMessage};
use super::state::AppState;
use crate::auth::jwt;
use crate::errors::ApiError;
use crate::metrics::WS_QUEUE_DROPPED;

/// Validates the JWT, binds the WS to the auth-keyed registry, and replies
/// with [`ServerMessage::Authenticated`]. Side effect: upon success, the
/// connection becomes addressable by `notify_user` for any auth-scoped
/// push (friend requests, DMs, ...).
pub async fn handle_authenticate(
    state: &Arc<AppState>,
    tx: &mpsc::Sender<String>,
    auth_user_id: &mut Option<String>,
    token: String,
) {
    let outcome = match jwt::decode_token(&token) {
        Ok(claims) => {
            // Replace any previous binding (eg. ghost ws after reconnect).
            if let Some(old) = auth_user_id.as_deref() {
                state.subscriptions.unbind_user(old);
            }
            state.subscriptions.bind_user(&claims.sub, tx.clone());
            *auth_user_id = Some(claims.sub.clone());
            ServerMessage::Authenticated {
                user_id: claims.sub,
                ok: true,
            }
        }
        Err(e) => {
            debug!("WS auth failed: {:?}", e);
            *auth_user_id = None;
            ServerMessage::Authenticated {
                user_id: String::new(),
                ok: false,
            }
        }
    };

    if let Some(payload) = serialize_message(&outcome) {
        if tx.try_send(payload).is_err() {
            WS_QUEUE_DROPPED.inc();
        }
    }
}

/// Routes an inbound [`super::models::ClientMessage::DmSend`]: validates
/// friendship, persists the message, fans it out to recipient + sender,
/// then ACKs the sender (or pushes a structured error frame).
pub async fn handle_dm_send(
    state: &Arc<AppState>,
    tx: &mpsc::Sender<String>,
    auth_user_id: Option<&str>,
    to_user_id: String,
    message: String,
    client_msg_id: Option<String>,
) {
    let Some(uid) = auth_user_id else {
        push_error(tx, "DM requires authentication").await;
        return;
    };

    match dm::send_dm(state, uid, &to_user_id, message, client_msg_id.clone()).await {
        Ok(entry) => {
            let ack = ServerMessage::DmAck {
                id: entry.id,
                client_msg_id,
                timestamp: entry.timestamp,
            };
            if let Some(payload) = serialize_message(&ack) {
                if tx.try_send(payload).is_err() {
                    WS_QUEUE_DROPPED.inc();
                }
            }
        }
        Err(err) => {
            debug!("dm.send failed: {:?}", err);
            let (code, message) = api_error_to_code(err);
            // The recipient won't get anything; tell the sender via a
            // structured RPC-shaped error frame addressed to itself.
            notify_user(
                state,
                uid,
                &ServerMessage::RpcResult {
                    request_id: client_msg_id.unwrap_or_else(|| "dm-send".into()),
                    result: None,
                    error: Some(RpcError {
                        code: code.into(),
                        message,
                    }),
                },
            )
            .await;
        }
    }
}

async fn push_error(tx: &mpsc::Sender<String>, message: &str) {
    let msg = ServerMessage::Error {
        message: message.into(),
    };
    if let Some(payload) = serialize_message(&msg) {
        if tx.try_send(payload).is_err() {
            WS_QUEUE_DROPPED.inc();
        }
    }
}

fn api_error_to_code(err: ApiError) -> (&'static str, String) {
    match err {
        ApiError::BadRequest(m) => ("bad-request", m),
        ApiError::NotFound(m) => ("not-found", m),
        ApiError::Conflict(m) => ("conflict", m),
        ApiError::Forbidden(m) => ("forbidden", m),
        ApiError::Unauthorized(m) => ("unauthorized", m),
        other => ("internal", format!("{:?}", other)),
    }
}
