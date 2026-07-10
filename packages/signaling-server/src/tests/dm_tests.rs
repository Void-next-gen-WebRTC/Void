// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for the DM core: friendship gating, ring-buffer
//! history, and fan-out via the auth-keyed connection registry.

use std::collections::HashMap;
use std::sync::Arc;

use tempfile::tempdir;
use tokio::sync::{mpsc, RwLock};
use void_sfu::{Sfu, SfuConfig};

use crate::sfu::dm;
use crate::sfu::registry::ServerRegistry;
use crate::sfu::state::{AppState, WS_CHANNEL_CAPACITY};
use crate::sfu::subscriptions::Subscriptions;
use crate::store::{FriendRecord, Store, UserRecord};

fn user(id: &str) -> UserRecord {
    UserRecord {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        password_hash: None,
        avatar: None,
        public_key: Some(format!("pk-{id}")),
        created_at_ms: 0,
    }
}

fn friendship(id: &str, a: &str, b: &str, status: &str) -> FriendRecord {
    FriendRecord {
        id: id.into(),
        from_user_id: a.into(),
        to_user_id: b.into(),
        status: status.into(),
        created_at_ms: 0,
    }
}

fn build_state(users: &[&str], friends: &[(&str, &str, &str, &str)]) -> Arc<AppState> {
    // Tmp store path so we don't pollute the workspace.
    let tmp = tempdir().expect("tmp dir");
    let store_path = tmp.path().join("test_store.bin");
    let registry_path = tmp.path().join("test_servers.bin");
    std::mem::forget(tmp); // keep the dir alive for the test process
    let store = Store::load(store_path.to_str().unwrap());
    let server_registry = ServerRegistry::load(registry_path.to_str().unwrap());

    for u in users {
        store.users.insert((*u).into(), user(u));
    }
    for (id, a, b, s) in friends {
        store.friends.insert((*id).into(), friendship(id, a, b, s));
    }

    let sfu = Sfu::new(SfuConfig::default()).expect("sfu");
    Arc::new(AppState {
        peers: RwLock::new(HashMap::new()),
        chat_history: RwLock::new(HashMap::new()),
        dm_history: RwLock::new(HashMap::new()),
        server_registry,
        sfu,
        auth_store: store,
        subscriptions: Subscriptions::new(),
    })
}

fn bind(state: &Arc<AppState>, uid: &str) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>(WS_CHANNEL_CAPACITY);
    state.subscriptions.bind_user(uid, tx);
    rx
}

#[tokio::test]
async fn send_dm_fans_out_to_both_peers_and_persists_history() {
    let state = build_state(&["alice", "bob"], &[("f1", "alice", "bob", "accepted")]);
    let mut alice_rx = bind(&state, "alice");
    let mut bob_rx = bind(&state, "bob");

    let entry = dm::send_dm(&state, "alice", "bob", "hi bob".into(), Some("c-1".into()))
        .await
        .expect("send ok");

    // Both peers must receive the DmMessage frame.
    let alice_payload = alice_rx.try_recv().expect("alice received echo");
    let bob_payload = bob_rx.try_recv().expect("bob received message");
    for payload in [&alice_payload, &bob_payload] {
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["type"], "dm-message");
        assert_eq!(v["fromUserId"], "alice");
        assert_eq!(v["toUserId"], "bob");
        assert_eq!(v["message"], "hi bob");
        assert_eq!(v["id"], entry.id);
    }

    // Sender's echo carries the correlation id; recipient's payload omits it.
    let alice_v: serde_json::Value = serde_json::from_str(&alice_payload).unwrap();
    let bob_v: serde_json::Value = serde_json::from_str(&bob_payload).unwrap();
    assert_eq!(alice_v["clientMsgId"], "c-1");
    assert!(bob_v.get("clientMsgId").is_none());

    // History queryable from either side.
    let alice_hist = dm::dm_history(&state, "alice", "bob").await.unwrap();
    let bob_hist = dm::dm_history(&state, "bob", "alice").await.unwrap();
    assert_eq!(alice_hist.len(), 1);
    assert_eq!(bob_hist.len(), 1);
    assert_eq!(alice_hist[0].id, entry.id);
}

#[tokio::test]
async fn send_dm_rejects_non_friends() {
    let state = build_state(&["alice", "bob"], &[]);
    let _ = bind(&state, "alice");
    let err = dm::send_dm(&state, "alice", "bob", "spam".into(), None)
        .await
        .expect_err("must be forbidden");
    assert!(matches!(err, crate::errors::ApiError::Forbidden(_)));
}

#[tokio::test]
async fn send_dm_rejects_pending_friendship() {
    // Pending requests are not yet "accepted" — DM must be gated.
    let state = build_state(&["alice", "bob"], &[("f1", "alice", "bob", "pending")]);
    let err = dm::send_dm(&state, "alice", "bob", "hi".into(), None)
        .await
        .expect_err("must be forbidden");
    assert!(matches!(err, crate::errors::ApiError::Forbidden(_)));
}

#[tokio::test]
async fn dm_history_gated_by_friendship() {
    let state = build_state(&["alice", "bob"], &[]);
    let err = dm::dm_history(&state, "alice", "bob")
        .await
        .expect_err("forbidden");
    assert!(matches!(err, crate::errors::ApiError::Forbidden(_)));
}

#[tokio::test]
async fn notify_user_reaches_authenticated_socket_outside_voice() {
    // Reproduces the original bug: a user with no voice room must still
    // receive auth-scoped pushes. We use the friend-removed event as a
    // representative payload routed through `notify_user`.
    use crate::sfu::broadcast::notify_user;
    use crate::sfu::models::ServerMessage;

    let state = build_state(&["alice"], &[]);
    let mut rx = bind(&state, "alice");

    notify_user(
        &state,
        "alice",
        &ServerMessage::FriendRemoved {
            friendship_id: "f1".into(),
            by_user_id: "bob".into(),
        },
    )
    .await;

    let payload = rx.try_recv().expect("alice should be notified");
    let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(v["type"], "friend-removed");
    assert_eq!(v["byUserId"], "bob");
}

#[tokio::test]
async fn unbind_silences_subsequent_notifications() {
    use crate::sfu::broadcast::notify_user;
    use crate::sfu::models::ServerMessage;

    let state = build_state(&["alice"], &[]);
    let mut rx = bind(&state, "alice");
    state.subscriptions.unbind_user("alice");

    notify_user(
        &state,
        "alice",
        &ServerMessage::Error {
            message: "should not be delivered".into(),
        },
    )
    .await;

    assert!(rx.try_recv().is_err(), "no payload after unbind");
}
