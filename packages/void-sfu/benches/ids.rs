// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for the cheap-clone identifier newtypes.
//!
//! Run with:
//! ```bash
//! cargo bench -p void-sfu --bench ids
//! ```

use std::collections::HashMap;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use void_sfu::{DataChannelSourceId, MediaSourceId, PeerId};

const SAMPLE_PEER: &str = "550e8400-e29b-41d4-a716-446655440000";
const SAMPLE_TRACK: &str = "track-9f8e7d6c-5b4a-3210-fedc-ba9876543210";
const SAMPLE_LABEL: &str = "control";

fn bench_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("ids");

    group.bench_function("peer_id_from_str", |b| {
        b.iter(|| black_box(PeerId::from(black_box(SAMPLE_PEER))));
    });

    let pid = PeerId::from(SAMPLE_PEER);
    group.bench_function("peer_id_clone_arc_bump", |b| {
        b.iter(|| black_box(pid.clone()));
    });

    group.bench_function("media_source_id_from_peer_and_track", |b| {
        b.iter(|| {
            black_box(MediaSourceId::from_peer_and_track(
                black_box(&pid),
                black_box(SAMPLE_TRACK),
            ))
        });
    });

    group.bench_function("data_channel_source_id_from_peer_and_label", |b| {
        b.iter(|| {
            black_box(DataChannelSourceId::from_peer_and_label(
                black_box(&pid),
                black_box(SAMPLE_LABEL),
            ))
        });
    });

    group.finish();
}

fn bench_hashmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("ids");

    let ids: Vec<PeerId> = (0..1_000)
        .map(|i| PeerId::from(format!("peer-{i:08}")))
        .collect();

    group.bench_function("peer_id_hashmap_insert_lookup_1k", |b| {
        b.iter(|| {
            let mut map: HashMap<PeerId, u32> = HashMap::with_capacity(1_000);
            for (i, id) in ids.iter().enumerate() {
                map.insert(black_box(id.clone()), i as u32);
            }
            for id in ids.iter() {
                let _ = black_box(map.get(black_box(id)));
            }
        });
    });

    group.finish();
}

criterion_group!(benches, bench_construction, bench_hashmap);
criterion_main!(benches);
