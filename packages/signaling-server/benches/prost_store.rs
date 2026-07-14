// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for on-disk prost stores (`auth_store.bin`,
//! `ban_store.bin`, `servers.bin`).
//!
//! Run with:
//! ```bash
//! cargo bench -p signaling-server --bench prost_store
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use prost::Message;

use signaling_server::fraud::store::{BanRecord, BanSnapshot, FingerprintRecord, RecidivismRecord};
use signaling_server::gateway::registry::{ChannelRecord, ServerRecord, ServerSnapshot};
use signaling_server::store::{FriendRecord, StoreSnapshot, UserRecord};

fn user_record(i: usize) -> UserRecord {
    UserRecord {
        id: format!("550e8400-e29b-41d4-a716-{:012x}", i),
        username: format!("user{i}"),
        display_name: format!("User {i}"),
        password_hash: None,
        avatar: Some("https://cdn.example.com/a.png".into()),
        public_key: Some(format!("pk-{:0>56}", i)),
        created_at_ms: 1_700_000_000_000 + i as i64,
    }
}

fn friend_record(i: usize) -> FriendRecord {
    FriendRecord {
        id: format!("friend-{i:08}"),
        from_user_id: format!("u-{i:08}"),
        to_user_id: format!("u-{:08}", i + 1),
        status: "accepted".into(),
        created_at_ms: 1_700_000_000_000 + i as i64,
    }
}

fn ban_record(i: usize) -> BanRecord {
    BanRecord {
        ip: format!("10.{}.{}.{}", (i >> 16) & 0xff, (i >> 8) & 0xff, i & 0xff),
        reason: "login_bruteforce".into(),
        banned_at_ms: 1_700_000_000_000,
        expires_at_ms: 1_700_086_400_000,
    }
}

fn server_record(i: usize) -> ServerRecord {
    ServerRecord {
        id: format!("srv-{i:08}"),
        name: format!("Server {i}"),
        owner_public_key: format!("pk-{:0>56}", i),
        invite_key: format!("inv-{i:08}"),
        icon: Some("icon.png".into()),
        channels: (0..10)
            .map(|c| ChannelRecord {
                id: format!("chan-{i:04}-{c:02}"),
                name: format!("channel-{c}"),
                r#type: if c % 2 == 0 {
                    "text".into()
                } else {
                    "voice".into()
                },
            })
            .collect(),
        members: (0..200).map(|m| format!("pk-{:0>56}", m)).collect(),
    }
}

fn bench_store_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("prost_store");
    for &n in &[100usize, 1_000, 10_000] {
        let snap = StoreSnapshot {
            users: (0..n).map(user_record).collect(),
            friends: (0..n / 2).map(friend_record).collect(),
        };
        let bytes = snap.encode_to_vec();
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("store_snapshot_encode", n),
            &snap,
            |b, s| b.iter(|| black_box(s.encode_to_vec())),
        );
        group.bench_with_input(
            BenchmarkId::new("store_snapshot_decode", n),
            &bytes,
            |b, buf| {
                b.iter(|| {
                    let _ = StoreSnapshot::decode(black_box(buf.as_slice())).unwrap();
                });
            },
        );
    }
    group.finish();
}

fn bench_ban_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("prost_store");
    for &n in &[100usize, 10_000] {
        let snap = BanSnapshot {
            bans: (0..n).map(ban_record).collect(),
            recidivism: (0..n / 4)
                .map(|i| RecidivismRecord {
                    ip: format!("10.0.0.{}", i & 0xff),
                    ban_timestamps_ms: vec![1_700_000_000_000, 1_700_000_100_000],
                })
                .collect(),
            fingerprints: (0..n / 8)
                .map(|i| FingerprintRecord {
                    fingerprint: format!("fp-{i:08}"),
                    ips: (0..3)
                        .map(|j| format!("192.168.{}.{}", i & 0xff, j))
                        .collect(),
                })
                .collect(),
        };
        let bytes = snap.encode_to_vec();
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::new("ban_snapshot_encode", n), &snap, |b, s| {
            b.iter(|| black_box(s.encode_to_vec()))
        });
        group.bench_with_input(
            BenchmarkId::new("ban_snapshot_decode", n),
            &bytes,
            |b, buf| {
                b.iter(|| {
                    let _ = BanSnapshot::decode(black_box(buf.as_slice())).unwrap();
                });
            },
        );
    }
    group.finish();
}

fn bench_server_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("prost_store");
    let snap = ServerSnapshot {
        servers: (0..50).map(server_record).collect(),
    };
    let bytes = snap.encode_to_vec();
    group.throughput(Throughput::Bytes(bytes.len() as u64));

    group.bench_function("server_snapshot_encode_50x10x200", |b| {
        b.iter(|| black_box(snap.encode_to_vec()));
    });
    group.bench_function("server_snapshot_decode_50x10x200", |b| {
        b.iter(|| {
            let _ = ServerSnapshot::decode(black_box(bytes.as_slice())).unwrap();
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_store_snapshot,
    bench_ban_snapshot,
    bench_server_snapshot,
);
criterion_main!(benches);
