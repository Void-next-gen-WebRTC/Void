use crate::gateway::broadcast::serialize_message;
use crate::gateway::models::{PeerInfo, ServerMessage};

// ---------------------------------------------------------------------------
// 1. serialize_message produces valid JSON for Joined
// ---------------------------------------------------------------------------

#[test]
fn serialize_joined_message() {
    let msg = ServerMessage::Joined {
        channel_id: "ch-1".into(),
        peers: vec![PeerInfo {
            user_id: "u1".into(),
            username: "Alice".into(),
            is_muted: false,
            is_deafened: false,
        }],
        started_at: 12345,
    };
    let json = serialize_message(&msg).expect("should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "joined");
    assert_eq!(parsed["channelId"], "ch-1");
}

// ---------------------------------------------------------------------------
// 2. serialize_message for PeerLeft
// ---------------------------------------------------------------------------

#[test]
fn serialize_peer_left() {
    let msg = ServerMessage::PeerLeft {
        channel_id: "ch-1".into(),
        user_id: "u1".into(),
    };
    let json = serialize_message(&msg).expect("should serialize");
    assert!(json.contains("peer-left"));
}

// ---------------------------------------------------------------------------
// 3. serialize_message for Error
// ---------------------------------------------------------------------------

#[test]
fn serialize_error_message() {
    let msg = ServerMessage::Error {
        message: "test error".into(),
    };
    let json = serialize_message(&msg).expect("should serialize");
    assert!(json.contains("test error"));
}

// ---------------------------------------------------------------------------
// 4. serialize_message for Stats
// ---------------------------------------------------------------------------

#[test]
fn serialize_stats_message() {
    let msg = ServerMessage::Stats {
        user_id: "u1".into(),
        bandwidth_bps: 64000,
    };
    let json = serialize_message(&msg).expect("should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["bandwidthBps"], 64000);
}

// ---------------------------------------------------------------------------
// 5. serialize_message for Chat
// ---------------------------------------------------------------------------

#[test]
fn serialize_chat_message() {
    let msg = ServerMessage::Chat {
        channel_id: "ch-1".into(),
        from: "u1".into(),
        username: "Alice".into(),
        message: "Hello".into(),
        timestamp: 999,
    };
    let json = serialize_message(&msg).expect("should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "chat");
    assert_eq!(parsed["message"], "Hello");
}

// ---------------------------------------------------------------------------
// 6. serialize_message for TrackMap
// ---------------------------------------------------------------------------

#[test]
fn serialize_track_map() {
    let msg = ServerMessage::TrackMap {
        user_id: "u1".into(),
        track_id: "t1".into(),
        stream_id: "s1".into(),
        kind: "audio".into(),
    };
    let json = serialize_message(&msg).expect("should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "track-map");
}

// ---------------------------------------------------------------------------
// 7. serialize_message for Ice
// ---------------------------------------------------------------------------

#[test]
fn serialize_ice_message() {
    let msg = ServerMessage::Ice {
        candidate: serde_json::json!({"candidate": "c1", "sdpMid": "0"}),
    };
    let json = serialize_message(&msg).expect("should serialize");
    assert!(json.contains("ice"));
}
