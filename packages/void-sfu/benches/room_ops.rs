// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for room/peer membership operations.
//!
//! Run with:
//! ```bash
//! cargo bench -p void-sfu --bench room_ops
//! ```
//!
//! These benches exercise the registry-only paths of [`Sfu`] (add_peer,
//! join_room, leave_room, remove_peer, room_members, metrics_snapshot).
//! The actual RTP forwarding (`negotiation::on_track`) requires a live
//! `RTCPeerConnection` and is not benchable in isolation — see the README
//! for the rationale.

use std::sync::Arc;

use async_trait::async_trait;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::runtime::Runtime;

use void_sfu::{Outbound, PeerId, RoomId, Sfu, SfuConfig, SfuResult, SignalSink};

/// Minimal no-op sink: does nothing on delivery, used to satisfy the
/// `add_peer` contract without spinning up a real transport.
struct NullSink;

#[async_trait]
impl SignalSink for NullSink {
    async fn deliver(&self, _peer: &PeerId, _message: Outbound) -> SfuResult<()> {
        Ok(())
    }
}

fn build_sfu() -> Sfu {
    Sfu::new(SfuConfig::default()).expect("sfu init")
}

fn populate(rt: &Runtime, sfu: &Sfu, room: &RoomId, n: usize) {
    rt.block_on(async {
        for i in 0..n {
            let pid = PeerId::from(format!("seed-{i:08}"));
            sfu.add_peer(pid.clone(), Arc::new(NullSink)).unwrap();
            sfu.join_room(&pid, room.clone()).await.unwrap();
        }
    });
}

fn bench_add_peer(c: &mut Criterion) {
    let mut group = c.benchmark_group("room_ops");
    let rt = Runtime::new().unwrap();
    let sfu = build_sfu();

    // Hot path: `add_peer` is called once per WS handshake.
    let mut counter: u64 = 0;
    group.bench_function("sfu_add_peer", |b| {
        b.iter(|| {
            counter = counter.wrapping_add(1);
            let pid = PeerId::from(format!("p-{counter:016x}"));
            sfu.add_peer(black_box(pid), Arc::new(NullSink)).unwrap();
        });
    });
    drop(rt);
    group.finish();
}

fn bench_join_room_fanout(c: &mut Criterion) {
    let mut group = c.benchmark_group("room_ops");
    let rt = Runtime::new().unwrap();

    // Pre-populate rooms of various sizes; benchmark the cost of one
    // additional joiner snapshotting the existing membership.
    for &n in &[1usize, 8, 64] {
        let sfu = build_sfu();
        let room = RoomId::from(format!("room-{n}"));
        populate(&rt, &sfu, &room, n);

        let joiner = PeerId::from("late-joiner");
        sfu.add_peer(joiner.clone(), Arc::new(NullSink)).unwrap();

        group.bench_with_input(BenchmarkId::new("sfu_join_room_existing", n), &n, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    let _ = sfu
                        .join_room(black_box(&joiner), black_box(room.clone()))
                        .await
                        .unwrap();
                    // Move the joiner back out so each iteration measures
                    // the same scenario (transition empty→join).
                    sfu.leave_room(&joiner).await.unwrap();
                });
            });
        });
    }
    group.finish();
}

fn bench_room_members_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("room_ops");
    let rt = Runtime::new().unwrap();
    let sfu = build_sfu();
    let room = RoomId::from("snapshot-room");
    populate(&rt, &sfu, &room, 64);

    group.bench_function("sfu_room_members_64", |b| {
        b.iter(|| {
            let v = sfu.room_members(black_box(&room));
            black_box(v);
        });
    });
    group.finish();
}

fn bench_metrics_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("room_ops");
    let rt = Runtime::new().unwrap();
    let sfu = build_sfu();

    // 50 rooms × 32 peers each ~ realistic mid-sized server.
    rt.block_on(async {
        for r in 0..50 {
            let room = RoomId::from(format!("room-{r:03}"));
            for p in 0..32 {
                let pid = PeerId::from(format!("p-{r:03}-{p:03}"));
                sfu.add_peer(pid.clone(), Arc::new(NullSink)).unwrap();
                sfu.join_room(&pid, room.clone()).await.unwrap();
            }
        }
    });

    group.bench_function("sfu_metrics_snapshot_50x32", |b| {
        b.iter(|| {
            rt.block_on(async {
                let snap = sfu.metrics_snapshot().await;
                black_box(snap);
            });
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_add_peer,
    bench_join_room_fanout,
    bench_room_members_snapshot,
    bench_metrics_snapshot,
);
criterion_main!(benches);
