use std::sync::Arc;

use void_sfu::{PeerId, RoomId};

use super::models::ServerMessage;
use super::state::AppState;
use crate::metrics::WS_QUEUE_DROPPED;

/// Serializes a server message to JSON.
pub fn serialize_message(message: &ServerMessage) -> Option<String> {
    serde_json::to_string(message).ok()
}

/// Pushes a server message to a single user (by id) if currently connected.
///
/// Targets the auth-keyed `subscriptions.connections` registry — populated
/// at WS authentication and persistent for the lifetime of the socket,
/// regardless of voice-room membership. Drops are accounted for in
/// `WS_QUEUE_DROPPED`. No-op when the user is offline.
pub async fn notify_user(state: &Arc<AppState>, user_id: &str, message: &ServerMessage) {
    let payload = match serialize_message(message) {
        Some(p) => p,
        None => return,
    };
    state.subscriptions.send_to_user(user_id, &payload);
}

/// Broadcasts a JSON payload to every member of a (voice) channel.
///
/// Membership is queried from the SFU room state — the host no longer keeps
/// its own mirror. For text-channel broadcasts (no SFU room), use the
/// dedicated text subscribers map (introduced in phase 3).
pub async fn broadcast_to_channel(
    state: &Arc<AppState>,
    channel_id: &str,
    message: &ServerMessage,
    exclude: Option<&str>,
) {
    let payload = match serialize_message(message) {
        Some(p) => p,
        None => return,
    };

    let room_id = RoomId::from(channel_id);
    let members = state.sfu.room_members(&room_id);
    if members.is_empty() {
        return;
    }

    let peers = state.peers.read().await;
    for member in members {
        if exclude == Some(member.as_str()) {
            continue;
        }
        if let Some(peer) = peers.get(member.as_str()) {
            if peer.tx.try_send(payload.clone()).is_err() {
                WS_QUEUE_DROPPED.inc();
            }
        }
    }
}

/// Cleans up a peer: instructs the SFU to remove it (which closes its PC,
/// drops forwarders and notifies the room observer), then removes the
/// host-side metadata. Idempotent.
pub async fn remove_peer(state: &Arc<AppState>, user_id: &str) {
    let peer_id = PeerId::from(user_id);
    if let Err(e) = state.sfu.remove_peer(&peer_id).await {
        tracing::debug!("sfu.remove_peer({}): {:?}", user_id, e);
    }
    state.peers.write().await.remove(user_id);
}
