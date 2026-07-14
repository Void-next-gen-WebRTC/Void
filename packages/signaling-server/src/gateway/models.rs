use serde::{Deserialize, Serialize};

use crate::models::{PendingRequest, UserSummary};

// ---------------------------------------------------------------------------
// Server / Channel data types (used at runtime and in REST API)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerChannel {
    pub id: String,
    pub name: String,
    pub r#type: String,
}

/// Wire-compatible with the browser's `RTCIceServer` dictionary
/// (`urls` / `username` / `credential`), so the client can spread it
/// directly into `new RTCPeerConnection({ iceServers: [...] })`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub owner_public_key: String,
    pub invite_key: String,
    pub icon: Option<String>,
    pub channels: Vec<ServerChannel>,
    #[serde(default)]
    pub members: Vec<String>,
}

// ---------------------------------------------------------------------------
// WebSocket protocol — Client → Server
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientMessage {
    #[serde(rename_all = "camelCase")]
    Join {
        channel_id: String,
        user_id: String,
        username: String,
        fingerprint: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Leave { channel_id: String, user_id: String },
    #[serde(rename_all = "camelCase")]
    Offer { sdp: serde_json::Value },
    #[serde(rename_all = "camelCase")]
    Answer { sdp: serde_json::Value },
    #[serde(rename_all = "camelCase")]
    Ice { candidate: serde_json::Value },
    #[serde(rename_all = "camelCase")]
    MediaState {
        channel_id: String,
        user_id: String,
        is_muted: bool,
        is_deafened: bool,
    },
    #[serde(rename_all = "camelCase")]
    Chat {
        channel_id: String,
        from: String,
        username: String,
        message: String,
        timestamp: u64,
    },

    // -----------------------------------------------------------------------
    // WS-only flows (Phase 3) — replace per-feature REST polling
    // -----------------------------------------------------------------------
    /// Validates the JWT and binds the authenticated user_id to the WS
    /// connection. Required before issuing any RPC call.
    #[serde(rename_all = "camelCase")]
    Authenticate { token: String },

    /// Subscribes the WS to push events for a *text* channel. The server
    /// pushes new chat messages to every subscriber regardless of voice
    /// channel membership.
    #[serde(rename_all = "camelCase")]
    SubscribeChannel { channel_id: String },

    #[serde(rename_all = "camelCase")]
    UnsubscribeChannel { channel_id: String },

    /// Subscribes to server-level events: member join/leave + presence.
    #[serde(rename_all = "camelCase")]
    SubscribeServer { server_id: String },

    #[serde(rename_all = "camelCase")]
    UnsubscribeServer { server_id: String },

    /// Generic request/response envelope. The host routes `method` to the
    /// matching handler and replies with [`ServerMessage::RpcResult`] keyed
    /// by `request_id`.
    #[serde(rename_all = "camelCase")]
    Rpc {
        request_id: String,
        method: String,
        #[serde(default)]
        params: serde_json::Value,
    },

    /// Sends a 1-to-1 direct message to a peer the sender is friends with.
    /// Fan-out is performed server-side via `notify_user`; both the sender
    /// (echo) and the recipient receive a [`ServerMessage::DmMessage`].
    /// `clientMsgId` is echoed back in [`ServerMessage::DmAck`] so the UI
    /// can resolve its optimistic placeholder.
    #[serde(rename_all = "camelCase")]
    DmSend {
        to_user_id: String,
        message: String,
        #[serde(default)]
        client_msg_id: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// WebSocket protocol — Server → Client
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerMessage {
    #[serde(rename_all = "camelCase")]
    Joined {
        channel_id: String,
        peers: Vec<PeerInfo>,
        started_at: u64,
    },
    #[serde(rename_all = "camelCase")]
    PeerJoined {
        channel_id: String,
        peer: PeerInfo,
    },
    #[serde(rename_all = "camelCase")]
    PeerLeft {
        channel_id: String,
        user_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Answer {
        sdp: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    Offer {
        sdp: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    Ice {
        candidate: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    PeerState {
        channel_id: String,
        user_id: String,
        is_muted: bool,
        is_deafened: bool,
    },
    #[serde(rename_all = "camelCase")]
    TrackMap {
        user_id: String,
        track_id: String,
        stream_id: String,
        kind: String,
    },
    #[serde(rename_all = "camelCase")]
    Chat {
        channel_id: String,
        from: String,
        username: String,
        message: String,
        timestamp: u64,
    },
    #[serde(rename_all = "camelCase")]
    Stats {
        user_id: String,
        bandwidth_bps: u64,
    },
    Error {
        message: String,
    },

    // ---- Friend social events (unchanged) ----
    #[serde(rename_all = "camelCase")]
    FriendRequestReceived {
        request: PendingRequest,
    },
    #[serde(rename_all = "camelCase")]
    FriendRequestAccepted {
        request_id: String,
        friend: UserSummary,
    },
    #[serde(rename_all = "camelCase")]
    FriendRequestDeclined {
        request_id: String,
        by_user_id: String,
    },
    #[serde(rename_all = "camelCase")]
    FriendRequestCancelled {
        request_id: String,
        by_user_id: String,
    },
    #[serde(rename_all = "camelCase")]
    FriendRemoved {
        friendship_id: String,
        by_user_id: String,
    },

    // ---- Phase 3 WS-only events ----
    /// Acknowledges an `Authenticate` call. `ok = false` means the WS is
    /// still anonymous and must NOT issue authenticated RPCs.
    #[serde(rename_all = "camelCase")]
    Authenticated {
        user_id: String,
        ok: bool,
    },

    /// Pushed once right after a successful `Authenticate`. Carries the
    /// STUN servers (static) plus a freshly-minted, time-limited TURN
    /// credential (if a TURN deployment is configured) that the client
    /// merges into its `RTCPeerConnection` ICE server list. See
    /// `crate::turn` for the credential-minting scheme.
    #[serde(rename_all = "camelCase")]
    IceServers {
        servers: Vec<IceServerConfig>,
    },

    /// One member's online presence on a server changed.
    #[serde(rename_all = "camelCase")]
    ServerMemberPresence {
        server_id: String,
        user_id: String,
        online: bool,
    },

    /// A new member just joined a server (e.g. accepted invite).
    #[serde(rename_all = "camelCase")]
    ServerMemberAdded {
        server_id: String,
        member: UserSummary,
    },

    /// A member left or was removed.
    #[serde(rename_all = "camelCase")]
    ServerMemberRemoved {
        server_id: String,
        user_id: String,
    },

    /// Generic RPC reply matching a [`ClientMessage::Rpc`] by `request_id`.
    /// Exactly one of `result` / `error` is non-null.
    #[serde(rename_all = "camelCase")]
    RpcResult {
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },

    /// Direct message delivery — pushed to both the recipient and the
    /// sender (echo). Persisted in `state.dm_history` until process exit.
    /// `client_msg_id` is forwarded only on the sender's echo so the UI
    /// can deterministically resolve its optimistic placeholder without
    /// resorting to body-based heuristics.
    #[serde(rename_all = "camelCase")]
    DmMessage {
        id: String,
        from_user_id: String,
        to_user_id: String,
        message: String,
        timestamp: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        client_msg_id: Option<String>,
    },

    /// Acknowledges a [`ClientMessage::DmSend`] so the sender's UI can
    /// resolve its optimistic placeholder. Sent only to the original sender.
    #[serde(rename_all = "camelCase")]
    DmAck {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        client_msg_id: Option<String>,
        timestamp: u64,
    },
}

/// Structured RPC error. `code` is a stable string the client can branch on.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub user_id: String,
    pub username: String,
    pub is_muted: bool,
    pub is_deafened: bool,
}
