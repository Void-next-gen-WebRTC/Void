use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use dashmap::DashMap;
use prost::Message;
use tokio::sync::Notify;

use crate::sfu::models::{Server, ServerChannel};
use crate::sfu::registry::{ServerRegistry, ServerSnapshot};

/// Builds a minimal `Server` for testing purposes.
fn make_server(idx: usize) -> Server {
    Server {
        id: format!("srv-{idx}"),
        name: format!("Server {idx}"),
        owner_public_key: format!("pk-owner-{idx}"),
        invite_key: format!("inv-{idx}"),
        icon: None,
        channels: vec![ServerChannel {
            id: format!("ch-{idx}"),
            name: "general".into(),
            r#type: "text".into(),
        }],
        members: vec![format!("pk-owner-{idx}"), format!("pk-member-{idx}")],
    }
}

/// Creates an empty registry backed by a temp file.
fn temp_registry(dir: &std::path::Path) -> ServerRegistry {
    let path = dir.join("test_servers.bin");
    ServerRegistry {
        servers: Arc::new(DashMap::new()),
        member_index: Arc::new(DashMap::new()),
        dirty: Arc::new(Notify::new()),
        path: Arc::new(path.to_string_lossy().into_owned()),
    }
}

// ---------------------------------------------------------------------------
// 1. Protobuf round-trip: insert → flush → reload — data must be identical
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_single_server() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());

    let server = make_server(42);
    reg.servers.insert(server.id.clone(), server.clone());
    reg.flush().expect("flush");

    let reloaded =
        ServerRegistry::try_load_bin(&dir.path().join("test_servers.bin").to_string_lossy())
            .expect("reload");

    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].id, server.id);
    assert_eq!(reloaded[0].name, server.name);
    assert_eq!(reloaded[0].channels.len(), 1);
    assert_eq!(reloaded[0].members, server.members);
}

// ---------------------------------------------------------------------------
// 2. 1000 concurrent inserts + flush → binary file integrity
// ---------------------------------------------------------------------------

#[test]
fn concurrent_1000_inserts_then_flush_integrity() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());
    let reg_arc = Arc::new(reg);

    let handles: Vec<_> = (0..1000)
        .map(|i| {
            let r = Arc::clone(&reg_arc);
            thread::spawn(move || {
                let s = make_server(i);
                for pk in &s.members {
                    r.index_member(pk, &s.id);
                }
                r.servers.insert(s.id.clone(), s);
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread join");
    }

    assert_eq!(reg_arc.servers.len(), 1000);
    reg_arc.flush().expect("flush after 1000 inserts");

    let path_str = dir
        .path()
        .join("test_servers.bin")
        .to_string_lossy()
        .into_owned();
    let reloaded = ServerRegistry::try_load_bin(&path_str).expect("reload");
    assert_eq!(reloaded.len(), 1000);

    for i in 0..1000 {
        let expected_id = format!("srv-{i}");
        assert!(
            reloaded.iter().any(|s| s.id == expected_id),
            "Missing server {expected_id} after reload"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. 10 writer threads × 100 servers + 5 concurrent flushers
// ---------------------------------------------------------------------------

#[test]
fn concurrent_writes_and_flushes() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = Arc::new(temp_registry(dir.path()));
    let flush_errors = Arc::new(AtomicUsize::new(0));

    let mut handles: Vec<thread::JoinHandle<()>> = (0..10)
        .map(|t| {
            let r = Arc::clone(&reg);
            thread::spawn(move || {
                for i in 0..100 {
                    let idx = t * 100 + i;
                    let s = make_server(idx);
                    for pk in &s.members {
                        r.index_member(pk, &s.id);
                    }
                    r.servers.insert(s.id.clone(), s);
                }
            })
        })
        .collect();

    for _ in 0..5 {
        let r = Arc::clone(&reg);
        let errs = Arc::clone(&flush_errors);
        handles.push(thread::spawn(move || {
            for _ in 0..20 {
                if r.flush().is_err() {
                    errs.fetch_add(1, Ordering::Relaxed);
                }
                thread::sleep(std::time::Duration::from_micros(100));
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join");
    }

    reg.flush().expect("final flush");

    let path_str = dir
        .path()
        .join("test_servers.bin")
        .to_string_lossy()
        .into_owned();
    let reloaded = ServerRegistry::try_load_bin(&path_str).expect("reload");
    assert_eq!(
        reloaded.len(),
        1000,
        "Expected 1000 servers after concurrent writes+flushes"
    );

    let raw = std::fs::read(dir.path().join("test_servers.bin")).expect("read bin");
    let snap = ServerSnapshot::decode(raw.as_slice()).expect("protobuf decode must succeed");
    assert_eq!(snap.servers.len(), 1000);
}

// ---------------------------------------------------------------------------
// 4. Member index integrity after 1000 concurrent inserts
// ---------------------------------------------------------------------------

#[test]
fn member_index_integrity_under_load() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = Arc::new(temp_registry(dir.path()));

    let handles: Vec<_> = (0..1000)
        .map(|i| {
            let r = Arc::clone(&reg);
            thread::spawn(move || {
                let s = make_server(i);
                for pk in &s.members {
                    r.index_member(pk, &s.id);
                }
                r.servers.insert(s.id.clone(), s);
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread join");
    }

    for i in 0..1000 {
        let owner_pk = format!("pk-owner-{i}");
        let member_pk = format!("pk-member-{i}");
        let server_id = format!("srv-{i}");

        let owner_entry = reg.member_index.get(&owner_pk).expect("owner pk in index");
        assert!(
            owner_entry.contains(&server_id),
            "Owner index missing {server_id}"
        );

        let member_entry = reg
            .member_index
            .get(&member_pk)
            .expect("member pk in index");
        assert!(
            member_entry.contains(&server_id),
            "Member index missing {server_id}"
        );
    }

    assert_eq!(reg.member_index.len(), 2000, "2 unique pks × 1000 servers");
}

// ---------------------------------------------------------------------------
// 5. Atomic write safety: no leftover .bin.tmp after successful flush
// ---------------------------------------------------------------------------

#[test]
fn no_leftover_tmp_file_after_flush() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let reg = temp_registry(dir.path());

    reg.servers.insert("s1".into(), make_server(1));
    reg.flush().expect("flush");

    let tmp_path = dir.path().join("test_servers.bin.tmp");
    assert!(
        !tmp_path.exists(),
        ".bin.tmp should be removed after successful flush"
    );
    assert!(
        dir.path().join("test_servers.bin").exists(),
        ".bin must exist"
    );
}

// ---------------------------------------------------------------------------
// 6. Loading from empty / non-existent file yields empty registry
// ---------------------------------------------------------------------------

#[test]
fn load_nonexistent_file_yields_empty() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = dir.path().join("does_not_exist.bin");
    let reg = ServerRegistry::load(&path.to_string_lossy());
    assert_eq!(reg.servers.len(), 0);
}

// ---------------------------------------------------------------------------
// 7. Repeated flush + reload cycle stays consistent
// ---------------------------------------------------------------------------

#[test]
fn repeated_flush_reload_consistency() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path_str = dir.path().join("cycle.bin").to_string_lossy().into_owned();

    for round in 0..10 {
        let reg = ServerRegistry::load(&path_str);

        let base = round * 50;
        for i in base..(base + 50) {
            let s = make_server(i);
            for pk in &s.members {
                reg.index_member(pk, &s.id);
            }
            reg.servers.insert(s.id.clone(), s);
        }
        reg.flush().expect("flush");
    }

    let final_reg = ServerRegistry::load(&path_str);
    assert_eq!(final_reg.servers.len(), 500, "10 rounds × 50 servers");
}
