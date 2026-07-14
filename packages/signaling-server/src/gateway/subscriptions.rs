//! WS-only push state: text-channel chat subscriptions and server-level
//! presence subscriptions. Kept outside [`AppState`] in a dedicated handle
//! so it can be cloned freely into background tasks (zero global RwLock).
//!
//! All maps are concurrent ([`DashMap`] / [`DashSet`]); no `await` is held
//! across modifications.

use std::sync::Arc;

use dashmap::{DashMap, DashSet};
use tokio::sync::mpsc;

use super::broadcast::serialize_message;
use super::models::ServerMessage;
use super::state::AppState;
use crate::metrics::WS_QUEUE_DROPPED;

/// Subscription registry. One instance lives in [`AppState`].
#[derive(Default)]
pub struct Subscriptions {
    /// Text-channel chat subscribers: channel_id -> set of user_ids.
    pub channel_subscribers: DashMap<String, DashSet<String>>,
    /// Server presence subscribers: server_id -> set of user_ids.
    pub server_subscribers: DashMap<String, DashSet<String>>,
    /// Reverse index: which servers a user is subscribed to. Used to
    /// broadcast presence on disconnect without scanning every server.
    pub user_to_servers: DashMap<String, DashSet<String>>,
    /// Authenticated WebSocket connections, keyed by `auth_user_id`.
    /// Populated at `Authenticate`, cleared on socket close. This is the
    /// canonical sink for any auth-scoped push (friend requests, DMs,
    /// channel/server subscriptions) — `state.peers` is voice-only and
    /// would silently drop notifications outside of voice rooms.
    pub connections: DashMap<String, mpsc::Sender<String>>,
}

impl Subscriptions {
    #[inline]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Binds an authenticated user to its WS sender. Replaces any previous
    /// binding (typically a stale ghost connection).
    pub fn bind_user(&self, user_id: &str, tx: mpsc::Sender<String>) {
        self.connections.insert(user_id.to_string(), tx);
    }

    /// Drops the WS binding for a user.
    pub fn unbind_user(&self, user_id: &str) {
        self.connections.remove(user_id);
    }

    /// Pushes a pre-serialized JSON payload to `user_id`'s WS, accounting
    /// drops in the `WS_QUEUE_DROPPED` Prometheus counter. No-op if the
    /// user is not currently connected.
    pub fn send_to_user(&self, user_id: &str, payload: &str) {
        if let Some(entry) = self.connections.get(user_id) {
            if entry.value().try_send(payload.to_string()).is_err() {
                WS_QUEUE_DROPPED.inc();
            }
        }
    }

    pub fn subscribe_channel(&self, channel_id: &str, user_id: &str) {
        self.channel_subscribers
            .entry(channel_id.to_string())
            .or_default()
            .insert(user_id.to_string());
    }

    pub fn unsubscribe_channel(&self, channel_id: &str, user_id: &str) {
        if let Some(set) = self.channel_subscribers.get(channel_id) {
            set.remove(user_id);
        }
    }

    pub fn subscribe_server(&self, server_id: &str, user_id: &str) {
        self.server_subscribers
            .entry(server_id.to_string())
            .or_default()
            .insert(user_id.to_string());
        self.user_to_servers
            .entry(user_id.to_string())
            .or_default()
            .insert(server_id.to_string());
    }

    pub fn unsubscribe_server(&self, server_id: &str, user_id: &str) {
        if let Some(set) = self.server_subscribers.get(server_id) {
            set.remove(user_id);
        }
        if let Some(set) = self.user_to_servers.get(user_id) {
            set.remove(server_id);
        }
    }

    /// Removes every subscription owned by `user_id`. Returns the list of
    /// servers the user was subscribed to (for presence broadcast).
    pub fn drop_user(&self, user_id: &str) -> Vec<String> {
        for kv in self.channel_subscribers.iter() {
            kv.value().remove(user_id);
        }
        let servers: Vec<String> = self
            .user_to_servers
            .remove(user_id)
            .map(|(_, set)| set.iter().map(|s| s.clone()).collect())
            .unwrap_or_default();
        for sid in &servers {
            if let Some(set) = self.server_subscribers.get(sid) {
                set.remove(user_id);
            }
        }
        servers
    }

    /// Snapshot of channel subscribers (cheap clones — refcount-only `String`).
    pub fn channel_subscribers_snapshot(&self, channel_id: &str) -> Vec<String> {
        self.channel_subscribers
            .get(channel_id)
            .map(|set| set.iter().map(|u| u.clone()).collect())
            .unwrap_or_default()
    }

    /// Snapshot of server subscribers.
    pub fn server_subscribers_snapshot(&self, server_id: &str) -> Vec<String> {
        self.server_subscribers
            .get(server_id)
            .map(|set| set.iter().map(|u| u.clone()).collect())
            .unwrap_or_default()
    }
}

/// Pushes `message` to every subscriber of `channel_id`, except `exclude`.
///
/// Routes via the auth-keyed `connections` registry so that subscribers
/// receive the push even when they are not in a voice room (the legacy
/// `state.peers` map covers voice only).
pub async fn push_to_channel_subscribers(
    state: &Arc<AppState>,
    channel_id: &str,
    message: &ServerMessage,
    exclude: Option<&str>,
) {
    let payload = match serialize_message(message) {
        Some(p) => p,
        None => return,
    };
    let subs = state.subscriptions.channel_subscribers_snapshot(channel_id);
    for uid in subs {
        if exclude == Some(uid.as_str()) {
            continue;
        }
        state.subscriptions.send_to_user(&uid, &payload);
    }
}

/// Pushes `message` to every subscriber of `server_id`.
pub async fn push_to_server_subscribers(
    state: &Arc<AppState>,
    server_id: &str,
    message: &ServerMessage,
    exclude: Option<&str>,
) {
    let payload = match serialize_message(message) {
        Some(p) => p,
        None => return,
    };
    let subs = state.subscriptions.server_subscribers_snapshot(server_id);
    for uid in subs {
        if exclude == Some(uid.as_str()) {
            continue;
        }
        state.subscriptions.send_to_user(&uid, &payload);
    }
}
