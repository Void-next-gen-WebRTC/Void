// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Direct-message (1-to-1) core logic: friendship gating, in-RAM ring
//! buffer history, fan-out via [`notify_user`].
//!
//! The wire protocol lives in [`super::models`] (`ClientMessage::DmSend`,
//! `ServerMessage::DmMessage`, `ServerMessage::DmAck`). DMs travel on the
//! same authenticated WebSocket as the rest of the control plane — the
//! voice/video pipelines stay untouched (WebRTC handles media directly,
//! independently of this socket).

use std::collections::VecDeque;
use std::sync::Arc;

use uuid::Uuid;

use super::broadcast::notify_user;
use super::models::ServerMessage;
use super::state::{AppState, DmEntry, DmPairKey, DM_HISTORY_CAP};
use crate::errors::ApiError;
use crate::models::UserSummary;

/// Returns true if `a` and `b` have an `accepted` friendship in the store.
fn are_friends(state: &AppState, a: &str, b: &str) -> bool {
    state.auth_store.friends.iter().any(|r| {
        let f = r.value();
        f.status == "accepted"
            && ((f.from_user_id == a && f.to_user_id == b)
                || (f.from_user_id == b && f.to_user_id == a))
    })
}

#[inline]
fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Sends a direct message from `from_user_id` to `to_user_id`.
///
/// Steps:
/// 1. Validate non-empty body and self-targeting.
/// 2. Check `accepted` friendship — refuses with [`ApiError::Forbidden`].
/// 3. Persist into the ring buffer (capped at [`DM_HISTORY_CAP`] per pair).
/// 4. Push [`ServerMessage::DmMessage`] to both peers (recipient + echo).
///    The echo to the sender carries `client_msg_id` so the UI can resolve
///    its optimistic placeholder deterministically; the recipient receives
///    a payload without that field (it is meaningless to them).
///
/// Returns the persisted [`DmEntry`] so callers can also send a structured
/// ACK back to the sender via [`ServerMessage::DmAck`].
pub async fn send_dm(
    state: &Arc<AppState>,
    from_user_id: &str,
    to_user_id: &str,
    message: String,
    client_msg_id: Option<String>,
) -> Result<DmEntry, ApiError> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest("Empty message".into()));
    }
    if from_user_id == to_user_id {
        return Err(ApiError::BadRequest("Cannot DM yourself".into()));
    }
    if !state.auth_store.users.contains_key(to_user_id) {
        return Err(ApiError::NotFound("Recipient not found".into()));
    }
    if !are_friends(state, from_user_id, to_user_id) {
        return Err(ApiError::Forbidden(
            "You can only DM accepted friends".into(),
        ));
    }

    let entry = DmEntry {
        id: Uuid::new_v4().to_string(),
        from_user_id: from_user_id.to_string(),
        to_user_id: to_user_id.to_string(),
        message: trimmed.to_string(),
        timestamp: epoch_ms(),
    };

    {
        let mut history = state.dm_history.write().await;
        let buf = history
            .entry(DmPairKey::new(from_user_id, to_user_id))
            .or_insert_with(|| VecDeque::with_capacity(DM_HISTORY_CAP));
        if buf.len() >= DM_HISTORY_CAP {
            buf.pop_front();
        }
        buf.push_back(entry.clone());
    }

    // Recipient payload: no `client_msg_id` (sender's correlation token).
    let recipient_payload = ServerMessage::DmMessage {
        id: entry.id.clone(),
        from_user_id: entry.from_user_id.clone(),
        to_user_id: entry.to_user_id.clone(),
        message: entry.message.clone(),
        timestamp: entry.timestamp,
        client_msg_id: None,
    };
    notify_user(state, to_user_id, &recipient_payload).await;

    // Sender echo: include the correlation id so the UI replaces the
    // optimistic placeholder instead of appending a duplicate bubble.
    let sender_payload = ServerMessage::DmMessage {
        id: entry.id.clone(),
        from_user_id: entry.from_user_id.clone(),
        to_user_id: entry.to_user_id.clone(),
        message: entry.message.clone(),
        timestamp: entry.timestamp,
        client_msg_id,
    };
    notify_user(state, from_user_id, &sender_payload).await;

    Ok(entry)
}

/// Returns the message history between `user_id` and `with_user_id`, oldest
/// first. Empty vec if no messages exist or if the pair is not friends
/// (history is gated by the same friendship rule as `send_dm`).
pub async fn dm_history(
    state: &Arc<AppState>,
    user_id: &str,
    with_user_id: &str,
) -> Result<Vec<DmEntry>, ApiError> {
    if user_id == with_user_id {
        return Ok(Vec::new());
    }
    if !are_friends(state, user_id, with_user_id) {
        return Err(ApiError::Forbidden(
            "You can only read DMs with accepted friends".into(),
        ));
    }
    let history = state.dm_history.read().await;
    let entries: Vec<DmEntry> = history
        .get(&DmPairKey::new(user_id, with_user_id))
        .map(|buf| buf.iter().cloned().collect())
        .unwrap_or_default();
    Ok(entries)
}

/// Lists every friend the user has exchanged at least one DM with, in
/// most-recent-first order. Useful to populate the "Recent DMs" sidebar.
pub async fn list_recent_dm_partners(state: &Arc<AppState>, user_id: &str) -> Vec<UserSummary> {
    let mut partners: Vec<(String, u64)> = Vec::new();
    {
        let history = state.dm_history.read().await;
        for (key, buf) in history.iter() {
            let other = if key.0 == user_id {
                Some(&key.1)
            } else if key.1 == user_id {
                Some(&key.0)
            } else {
                None
            };
            if let Some(o) = other {
                let last_ts = buf.back().map(|e| e.timestamp).unwrap_or(0);
                partners.push((o.clone(), last_ts));
            }
        }
    }
    partners.sort_by_key(|a| std::cmp::Reverse(a.1));
    partners
        .into_iter()
        .filter_map(|(uid, _)| {
            state
                .auth_store
                .users
                .get(&uid)
                .map(|u| UserSummary::from(u.value()))
        })
        .collect()
}
