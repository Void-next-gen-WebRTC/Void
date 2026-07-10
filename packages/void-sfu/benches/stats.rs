// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for the per-destination forwarding stats counter.
//!
//! Run with:
//! ```bash
//! cargo bench -p void-sfu --bench stats
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use void_sfu::ForwardingStats;

fn bench_stats(c: &mut Criterion) {
    let mut group = c.benchmark_group("stats");

    // RTP write hot path: one update per forwarded packet.
    group.throughput(Throughput::Elements(1));
    group.bench_function("forwarding_stats_update", |b| {
        let mut s = ForwardingStats::new();
        b.iter(|| {
            s.update(black_box(1), black_box(1100));
        });
    });

    // Slow-tick aggregator: bandwidth read on a populated counter.
    group.bench_function("forwarding_stats_bandwidth_bps", |b| {
        let mut s = ForwardingStats::new();
        s.update(100_000, 100_000 * 1100);
        b.iter(|| black_box(s.bandwidth_bps()));
    });

    group.finish();
}

criterion_group!(benches, bench_stats);
criterion_main!(benches);
