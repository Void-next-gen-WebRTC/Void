use std::path::Path;
use std::sync::Arc;

use dashmap::DashMap;
use prost::Message;
use tokio::sync::Notify;

// ---------------------------------------------------------------------------
// Protobuf record types (prost derive — no .proto, no protoc)
// ---------------------------------------------------------------------------

/// Single user record serialized to disk as protobuf.
#[derive(Clone, PartialEq, prost::Message)]
pub struct UserRecord {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub username: String,
    #[prost(string, tag = "3")]
    pub display_name: String,
    // tag 4: legacy password_hash — kept optional for backward-compatible deserialization.
    // New records leave this empty; auth is Ed25519 nonce-challenge only.
    #[prost(string, optional, tag = "4")]
    pub password_hash: Option<String>,
    #[prost(string, optional, tag = "5")]
    pub avatar: Option<String>,
    #[prost(string, optional, tag = "6")]
    pub public_key: Option<String>,
    #[prost(int64, tag = "7")]
    pub created_at_ms: i64,
}

/// Single friend-link or pending request.
#[derive(Clone, PartialEq, prost::Message)]
pub struct FriendRecord {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub from_user_id: String,
    #[prost(string, tag = "3")]
    pub to_user_id: String,
    /// "pending" | "accepted" | "rejected"
    #[prost(string, tag = "4")]
    pub status: String,
    #[prost(int64, tag = "5")]
    pub created_at_ms: i64,
}

/// Root protobuf envelope written to a single `.bin` file.
#[derive(Clone, PartialEq, prost::Message)]
pub struct StoreSnapshot {
    #[prost(message, repeated, tag = "1")]
    pub users: Vec<UserRecord>,
    #[prost(message, repeated, tag = "2")]
    pub friends: Vec<FriendRecord>,
}

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

/// Concurrent in-memory store backed by DashMap.
/// Flushed periodically to a protobuf binary file on disk.
#[derive(Clone)]
pub struct Store {
    pub users: Arc<DashMap<String, UserRecord>>,
    /// Secondary index: `username (lowercased)` → `user id`.
    pub username_index: Arc<DashMap<String, String>>,
    /// Secondary index: `public_key` → `user id` for O(1) membership resolution.
    pub pubkey_index: Arc<DashMap<String, String>>,
    pub friends: Arc<DashMap<String, FriendRecord>>,
    pub(crate) dirty: Arc<Notify>,
    pub(crate) path: Arc<String>,
}

impl Store {
    /// Loads or creates the store from a `.bin` file.
    pub fn load(path: &str) -> Self {
        let (users, username_index, pubkey_index, friends) = Self::read_snapshot(path);
        Self {
            users,
            username_index,
            pubkey_index,
            friends,
            dirty: Arc::new(Notify::new()),
            path: Arc::new(path.to_string()),
        }
    }

    /// Marks the store as dirty so the background task flushes it.
    pub fn mark_dirty(&self) {
        self.dirty.notify_one();
    }

    /// Serializes the entire state to protobuf and writes to disk atomically.
    pub fn flush(&self) -> Result<(), String> {
        let users: Vec<UserRecord> = self.users.iter().map(|r| r.value().clone()).collect();
        let friends: Vec<FriendRecord> = self.friends.iter().map(|r| r.value().clone()).collect();

        let snapshot = StoreSnapshot { users, friends };
        let buf = snapshot.encode_to_vec();

        let path = Path::new(self.path.as_str());
        let tmp = path.with_extension("bin.tmp");
        std::fs::write(&tmp, &buf).map_err(|e| format!("write tmp: {e}"))?;

        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("remove old: {e}"))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))?;

        tracing::info!("Store flushed ({} bytes)", buf.len());
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn read_snapshot(
        path: &str,
    ) -> (
        Arc<DashMap<String, UserRecord>>,
        Arc<DashMap<String, String>>,
        Arc<DashMap<String, String>>,
        Arc<DashMap<String, FriendRecord>>,
    ) {
        let users = Arc::new(DashMap::new());
        let username_index = Arc::new(DashMap::new());
        let pubkey_index = Arc::new(DashMap::new());
        let friends = Arc::new(DashMap::new());

        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(snap) = StoreSnapshot::decode(bytes.as_slice()) {
                for u in snap.users {
                    username_index.insert(u.username.to_lowercase(), u.id.clone());
                    if let Some(ref pk) = u.public_key {
                        pubkey_index.insert(pk.clone(), u.id.clone());
                    }
                    users.insert(u.id.clone(), u);
                }
                for f in snap.friends {
                    friends.insert(f.id.clone(), f);
                }
                tracing::info!(
                    "Loaded store from disk ({} users, {} friend records)",
                    users.len(),
                    friends.len()
                );
            }
        }

        (users, username_index, pubkey_index, friends)
    }
}

/// Spawns a background task that flushes the store whenever marked dirty,
/// with a 2-second debounce to batch rapid writes.
/// Disk I/O runs on the blocking threadpool to avoid stalling async tasks.
pub fn spawn_flusher(store: Store) {
    tokio::spawn(async move {
        loop {
            store.dirty.notified().await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let store_ref = store.clone();
            match tokio::task::spawn_blocking(move || store_ref.flush()).await {
                Ok(Err(e)) => tracing::error!("Flush failed: {e}"),
                Err(e) => tracing::error!("Flush task panicked: {e}"),
                _ => {}
            }
        }
    });
}
