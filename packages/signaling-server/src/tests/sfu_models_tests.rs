use crate::gateway::models::{ClientMessage, PeerInfo, ServerMessage};

// ---------------------------------------------------------------------------
// ClientMessage deserialization
// ---------------------------------------------------------------------------

#[test]
fn deserialize_join() {
    let json = r#"{
        "type": "join",
        "channelId": "ch1",
        "userId": "u1",
        "username": "Alice",
        "fingerprint": "fp-abc"
    }"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    match msg {
        ClientMessage::Join {
            channel_id,
            user_id,
            username,
            fingerprint,
        } => {
            assert_eq!(channel_id, "ch1");
            assert_eq!(user_id, "u1");
            assert_eq!(username, "Alice");
            assert_eq!(fingerprint, Some("fp-abc".into()));
        }
        _ => panic!("expected Join"),
    }
}

#[test]
fn deserialize_join_no_fingerprint() {
    let json = r#"{
        "type": "join",
        "channelId": "ch1",
        "userId": "u1",
        "username": "Bob"
    }"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    match msg {
        ClientMessage::Join { fingerprint, .. } => assert!(fingerprint.is_none()),
        _ => panic!("expected Join"),
    }
}

#[test]
fn deserialize_leave() {
    let json = r#"{"type": "leave", "channelId": "ch1", "userId": "u1"}"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    assert!(matches!(msg, ClientMessage::Leave { .. }));
}

#[test]
fn deserialize_offer() {
    let json = r#"{"type": "offer", "sdp": {"sdp": "v=0\r\n..."}}"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    assert!(matches!(msg, ClientMessage::Offer { .. }));
}

#[test]
fn deserialize_answer() {
    let json = r#"{"type": "answer", "sdp": {"sdp": "v=0\r\n..."}}"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    assert!(matches!(msg, ClientMessage::Answer { .. }));
}

#[test]
fn deserialize_ice() {
    let json = r#"{"type": "ice", "candidate": {"candidate": "c", "sdpMid": "0"}}"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    assert!(matches!(msg, ClientMessage::Ice { .. }));
}

#[test]
fn deserialize_media_state() {
    let json = r#"{
        "type": "media-state",
        "channelId": "ch1",
        "userId": "u1",
        "isMuted": true,
        "isDeafened": false
    }"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    match msg {
        ClientMessage::MediaState {
            is_muted,
            is_deafened,
            ..
        } => {
            assert!(is_muted);
            assert!(!is_deafened);
        }
        _ => panic!("expected MediaState"),
    }
}

#[test]
fn deserialize_chat() {
    let json = r#"{
        "type": "chat",
        "channelId": "ch1",
        "from": "u1",
        "username": "Alice",
        "message": "Hello!",
        "timestamp": 1700000000
    }"#;
    let msg: ClientMessage = serde_json::from_str(json).unwrap();
    match msg {
        ClientMessage::Chat {
            message, timestamp, ..
        } => {
            assert_eq!(message, "Hello!");
            assert_eq!(timestamp, 1_700_000_000);
        }
        _ => panic!("expected Chat"),
    }
}

#[test]
fn invalid_type_fails() {
    let json = r#"{"type": "unknown", "data": 42}"#;
    let result = serde_json::from_str::<ClientMessage>(json);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// ServerMessage serialization
// ---------------------------------------------------------------------------

#[test]
fn serialize_joined() {
    let msg = ServerMessage::Joined {
        channel_id: "ch1".into(),
        peers: vec![PeerInfo {
            user_id: "u1".into(),
            username: "Alice".into(),
            is_muted: false,
            is_deafened: false,
        }],
        started_at: 1_700_000_000,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "joined");
    assert_eq!(json["peers"][0]["userId"], "u1");
}

#[test]
fn serialize_peer_left() {
    let msg = ServerMessage::PeerLeft {
        channel_id: "ch1".into(),
        user_id: "u1".into(),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "peer-left");
    assert_eq!(json["userId"], "u1");
}

#[test]
fn serialize_stats() {
    let msg = ServerMessage::Stats {
        user_id: "u1".into(),
        bandwidth_bps: 128_000,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "stats");
    assert_eq!(json["bandwidthBps"], 128_000);
}

#[test]
fn serialize_error() {
    let msg = ServerMessage::Error {
        message: "something broke".into(),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "error");
    assert_eq!(json["message"], "something broke");
}

#[test]
fn serialize_track_map() {
    let msg = ServerMessage::TrackMap {
        user_id: "u1".into(),
        track_id: "t1".into(),
        stream_id: "s1".into(),
        kind: "audio".into(),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "track-map");
    assert_eq!(json["kind"], "audio");
}

#[test]
fn serialize_peer_state() {
    let msg = ServerMessage::PeerState {
        channel_id: "ch1".into(),
        user_id: "u1".into(),
        is_muted: true,
        is_deafened: true,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "peer-state");
    assert!(json["isMuted"].as_bool().unwrap());
}

// ---------------------------------------------------------------------------
// PeerInfo
// ---------------------------------------------------------------------------

#[test]
fn peer_info_clone_and_serialize() {
    let pi = PeerInfo {
        user_id: "u1".into(),
        username: "Alice".into(),
        is_muted: false,
        is_deafened: true,
    };
    let cloned = pi.clone();
    assert_eq!(cloned.user_id, "u1");
    let json = serde_json::to_value(&cloned).unwrap();
    assert_eq!(json["isDeafened"], true);
}
