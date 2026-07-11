use std::path::Path;
use std::sync::Arc;

use dashmap::DashMap;
use prost::Message;
use tokio::sync::Notify;

use super::models::{Server, ServerChannel};

// ---------------------------------------------------------------------------
// Protobuf wire types (prost derive — no .proto file needed)
// ---------------------------------------------------------------------------

/// Protobuf representation of a channel within a server.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ChannelRecord {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub r#type: String,
}

/// Protobuf representation of a server (guild).
#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerRecord {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub owner_public_key: String,
    #[prost(string, tag = "4")]
    pub invite_key: String,
    #[prost(string, optional, tag = "5")]
    pub icon: Option<String>,
    #[prost(message, repeated, tag = "6")]
    pub channels: Vec<ChannelRecord>,
    #[prost(string, repeated, tag = "7")]
    pub members: Vec<String>,
}

/// Root envelope written to disk as a single `.bin` file.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerSnapshot {
    #[prost(message, repeated, tag = "1")]
    pub servers: Vec<ServerRecord>,
}

// ---------------------------------------------------------------------------
// Conversions between runtime (serde) types and protobuf records
// ---------------------------------------------------------------------------

impl From<&Server> for ServerRecord {
    fn from(s: &Server) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            owner_public_key: s.owner_public_key.clone(),
            invite_key: s.invite_key.clone(),
            icon: s.icon.clone(),
            channels: s.channels.iter().map(ChannelRecord::from).collect(),
            members: s.members.clone(),
        }
    }
}

impl From<&ServerChannel> for ChannelRecord {
    fn from(c: &ServerChannel) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            r#type: c.r#type.clone(),
        }
    }
}

impl From<&ServerRecord> for Server {
    fn from(r: &ServerRecord) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            owner_public_key: r.owner_public_key.clone(),
            invite_key: r.invite_key.clone(),
            icon: r.icon.clone(),
            channels: r.channels.iter().map(ServerChannel::from).collect(),
            members: r.members.clone(),
        }
    }
}

impl From<&ChannelRecord> for ServerChannel {
    fn from(r: &ChannelRecord) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            r#type: r.r#type.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// ServerRegistry — concurrent store backed by Protobuf binary file
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ServerRegistry {
    pub servers: Arc<DashMap<String, Server>>,
    /// Secondary index: `member_public_key` → list of `server_id`.
    pub member_index: Arc<DashMap<String, Vec<String>>>,
    pub(crate) dirty: Arc<Notify>,
    pub(crate) path: Arc<String>,
}

impl ServerRegistry {
    /// Loads the registry from a protobuf `.bin` file.
    pub fn load(path: &str) -> Self {
        let servers = Arc::new(DashMap::new());
        let member_index = Arc::new(DashMap::new());

        let loaded = Self::try_load_bin(path).unwrap_or_default();

        for s in &loaded {
            for member_pk in &s.members {
                member_index
                    .entry(member_pk.clone())
                    .or_insert_with(Vec::new)
                    .push(s.id.clone());
            }
            servers.insert(s.id.clone(), s.clone());
        }

        let registry = Self {
            servers,
            member_index,
            dirty: Arc::new(Notify::new()),
            path: Arc::new(path.to_string()),
        };

        tracing::info!("ServerRegistry loaded ({} servers)", registry.servers.len());
        registry
    }

    /// Signals the background flusher that data has changed.
    pub fn mark_dirty(&self) {
        self.dirty.notify_one();
    }

    /// Convenience alias: marks the registry dirty so the background flusher persists changes.
    pub fn save(&self) {
        self.mark_dirty();
    }

    /// Flushes the registry to disk **synchronously** (bypasses the debounced flusher).
    /// Reserved for critical mutations (e.g. public-key migration) where data loss
    /// on an immediate restart would cause permanent desync.
    pub fn flush_sync(&self) {
        if let Err(e) = self.flush() {
            tracing::error!("Synchronous ServerRegistry flush failed: {e}");
        }
    }

    /// Adds a member to the secondary index for a given server.
    pub fn index_member(&self, member_pk: &str, server_id: &str) {
        let mut entry = self
            .member_index
            .entry(member_pk.to_string())
            .or_default();
        if !entry.contains(&server_id.to_string()) {
            entry.push(server_id.to_string());
        }
    }

    /// Removes a server from all member indexes.
    pub fn remove_server_from_index(&self, server_id: &str) {
        self.member_index.iter_mut().for_each(|mut entry| {
            entry.value_mut().retain(|id| id != server_id);
        });
    }

    /// Serializes the entire state to protobuf and writes atomically.
    pub fn flush(&self) -> Result<(), String> {
        let records: Vec<ServerRecord> = self
            .servers
            .iter()
            .map(|kv| ServerRecord::from(kv.value()))
            .collect();

        let snapshot = ServerSnapshot { servers: records };
        let buf = snapshot.encode_to_vec();

        let path = Path::new(self.path.as_str());
        let tmp = path.with_extension("bin.tmp");
        std::fs::write(&tmp, &buf).map_err(|e| format!("write tmp: {e}"))?;
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("remove old: {e}"))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))?;

        tracing::info!(
            "ServerRegistry flushed ({} bytes, {} servers)",
            buf.len(),
            self.servers.len()
        );
        Ok(())
    }

    pub(crate) fn try_load_bin(path: &str) -> Option<Vec<Server>> {
        let bytes = std::fs::read(path).ok()?;
        let snap = ServerSnapshot::decode(bytes.as_slice()).ok()?;
        Some(snap.servers.iter().map(Server::from).collect())
    }
}

/// Spawns a debounced background flusher (2s) for the server registry.
/// Disk I/O runs on the blocking threadpool.
pub fn spawn_flusher(registry: ServerRegistry) {
    tokio::spawn(async move {
        loop {
            registry.dirty.notified().await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let registry_ref = registry.clone();
            match tokio::task::spawn_blocking(move || registry_ref.flush()).await {
                Ok(Err(e)) => tracing::error!("ServerRegistry flush failed: {e}"),
                Err(e) => tracing::error!("ServerRegistry flush task panicked: {e}"),
                _ => {}
            }
        }
    });
}
