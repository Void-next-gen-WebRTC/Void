use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Notify;

use crate::gateway::models::{Server, ServerChannel};
use crate::gateway::registry::{ChannelRecord, ServerRecord, ServerRegistry};

/// Creates an empty registry backed by a temp file.
fn temp_registry(dir: &std::path::Path) -> ServerRegistry {
    let path = dir.join("idx_servers.bin");
    ServerRegistry {
        servers: Arc::new(DashMap::new()),
        member_index: Arc::new(DashMap::new()),
        dirty: Arc::new(Notify::new()),
        path: Arc::new(path.to_string_lossy().into_owned()),
    }
}

fn make_server(idx: usize) -> Server {
    Server {
        id: format!("srv-{idx}"),
        name: format!("Server {idx}"),
        owner_public_key: format!("pk-{idx}"),
        invite_key: format!("inv-{idx}"),
        icon: Some("icon.png".into()),
        channels: vec![ServerChannel {
            id: format!("ch-{idx}"),
            name: "general".into(),
            r#type: "text".into(),
        }],
        members: vec![format!("pk-{idx}"), format!("pk-member-{idx}")],
    }
}

// ---------------------------------------------------------------------------
// 1. remove_server_from_index cleans all member entries
// ---------------------------------------------------------------------------

#[test]
fn remove_server_from_index_cleans() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());

    let s = make_server(1);
    for pk in &s.members {
        reg.index_member(pk, &s.id);
    }
    reg.servers.insert(s.id.clone(), s);

    assert!(!reg.member_index.is_empty());
    reg.remove_server_from_index("srv-1");

    // All entries for srv-1 should be gone
    for entry in reg.member_index.iter() {
        assert!(
            !entry.value().contains(&"srv-1".to_string()),
            "srv-1 should be removed from index"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. remove_server_from_index leaves other servers intact
// ---------------------------------------------------------------------------

#[test]
fn remove_preserves_other_servers() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());

    for i in 0..3 {
        let s = make_server(i);
        for pk in &s.members {
            reg.index_member(pk, &s.id);
        }
        reg.servers.insert(s.id.clone(), s);
    }

    reg.remove_server_from_index("srv-1");

    {
        let entry = reg.member_index.get("pk-0").expect("pk-0");
        assert!(entry.contains(&"srv-0".to_string()));
    }
    {
        let entry2 = reg.member_index.get("pk-2").expect("pk-2");
        assert!(entry2.contains(&"srv-2".to_string()));
    }
}

// ---------------------------------------------------------------------------
// 3. index_member is idempotent (no duplicates)
// ---------------------------------------------------------------------------

#[test]
fn index_member_idempotent() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());

    reg.index_member("pk-a", "srv-1");
    reg.index_member("pk-a", "srv-1");
    reg.index_member("pk-a", "srv-1");

    let entry = reg.member_index.get("pk-a").unwrap();
    assert_eq!(entry.value().len(), 1);
}

// ---------------------------------------------------------------------------
// 4. save / mark_dirty does not panic
// ---------------------------------------------------------------------------

#[test]
fn save_does_not_panic() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());
    reg.save(); // mark_dirty — no flusher running but shouldn't panic
}

// ---------------------------------------------------------------------------
// 5. flush_sync flushes to disk immediately
// ---------------------------------------------------------------------------

#[test]
fn flush_sync_writes_file() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());
    reg.servers.insert("srv-1".into(), make_server(1));
    reg.flush_sync();
    assert!(dir.path().join("idx_servers.bin").exists());
}

// ---------------------------------------------------------------------------
// 6. ServerRecord ↔ Server conversion round-trip
// ---------------------------------------------------------------------------

#[test]
fn server_record_roundtrip() {
    let s = make_server(42);
    let record = ServerRecord::from(&s);
    let back = Server::from(&record);

    assert_eq!(back.id, s.id);
    assert_eq!(back.name, s.name);
    assert_eq!(back.owner_public_key, s.owner_public_key);
    assert_eq!(back.invite_key, s.invite_key);
    assert_eq!(back.icon, s.icon);
    assert_eq!(back.channels.len(), s.channels.len());
    assert_eq!(back.members, s.members);
}

// ---------------------------------------------------------------------------
// 7. ChannelRecord ↔ ServerChannel conversion round-trip
// ---------------------------------------------------------------------------

#[test]
fn channel_record_roundtrip() {
    let ch = ServerChannel {
        id: "ch-1".into(),
        name: "general".into(),
        r#type: "voice".into(),
    };
    let record = ChannelRecord::from(&ch);
    let back = ServerChannel::from(&record);

    assert_eq!(back.id, ch.id);
    assert_eq!(back.name, ch.name);
    assert_eq!(back.r#type, ch.r#type);
}

// ---------------------------------------------------------------------------
// 8. Member shared across multiple servers
// ---------------------------------------------------------------------------

#[test]
fn member_in_multiple_servers() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());

    reg.index_member("pk-shared", "srv-1");
    reg.index_member("pk-shared", "srv-2");
    reg.index_member("pk-shared", "srv-3");

    {
        let entry = reg.member_index.get("pk-shared").unwrap();
        assert_eq!(entry.value().len(), 3);
    } // drop ref before remove_server_from_index iter_mut

    reg.remove_server_from_index("srv-2");

    {
        let entry = reg.member_index.get("pk-shared").unwrap();
        assert_eq!(entry.value().len(), 2);
        assert!(!entry.contains(&"srv-2".to_string()));
    }
}
