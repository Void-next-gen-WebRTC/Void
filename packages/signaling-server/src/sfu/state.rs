use super::registry::ServerRegistry;
use super::subscriptions::Subscriptions;
use crate::store::Store;
use crate::turn::TurnConfig;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use void_sfu::Sfu;
/// Max queued WebSocket JSON messages per peer before dropping.
pub const WS_CHANNEL_CAPACITY: usize = 512;
/// Max chat messages kept in-memory per channel.
pub const CHAT_HISTORY_CAP: usize = 200;
/// Max DM messages kept in-memory per user pair.
pub const DM_HISTORY_CAP: usize = 200;
/// Single persisted chat message kept in the in-memory ring buffer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEntry {
    pub id: String,
    pub channel_id: String,
    pub from: String,
    pub username: String,
    pub message: String,
    pub timestamp: u64,
}

/// Single direct message kept in the in-memory ring buffer keyed by the
/// ordered pair `(min_user_id, max_user_id)`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEntry {
    pub id: String,
    pub from_user_id: String,
    pub to_user_id: String,
    pub message: String,
    pub timestamp: u64,
}

/// Ordered key identifying a 1-to-1 conversation. Constructed via
/// [`DmPairKey::new`] which sorts the two user ids so both directions map
/// to the same bucket.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct DmPairKey(pub String, pub String);

impl DmPairKey {
    pub fn new(a: &str, b: &str) -> Self {
        if a <= b {
            Self(a.to_string(), b.to_string())
        } else {
            Self(b.to_string(), a.to_string())
        }
    }
}

/// Per-peer host metadata. All WebRTC state lives inside the `Sfu` instance.
#[derive(Clone)]
pub struct PeerSession {
    pub user_id: String,
    pub username: String,
    pub channel_id: String,
    pub tx: mpsc::Sender<String>,
    pub is_muted: bool,
    pub is_deafened: bool,
}
/// Shared application state available to all handlers.
pub struct AppState {
    pub peers: RwLock<HashMap<String, PeerSession>>,
    pub chat_history: RwLock<HashMap<String, VecDeque<ChatEntry>>>,
    /// In-RAM DM ring buffer. Keyed by ordered user pair so both directions
    /// land in the same bucket. No persistence in this iteration — survives
    /// only the lifetime of the process.
    pub dm_history: RwLock<HashMap<DmPairKey, VecDeque<DmEntry>>>,
    pub server_registry: ServerRegistry,
    pub sfu: Sfu,
    pub auth_store: Store,
    /// WS-only push subscriptions (text channels, server presence).
    pub subscriptions: Arc<Subscriptions>,
    /// `None` when no TURN server is configured (e.g. local dev) — clients
    /// then only receive STUN, which is insufficient behind symmetric/CGNAT
    /// networks.
    pub turn: Option<TurnConfig>,
}
