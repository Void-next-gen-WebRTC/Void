// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for the fraud detector and ban-store hot paths.
//!
//! Run with:
//! ```bash
//! cargo bench -p signaling-server --bench fraud
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tempfile::NamedTempFile;

use signaling_server::fraud::detector::FraudDetector;
use signaling_server::fraud::store::BanStore;
use signaling_server::nonce::NonceStore;

/// Builds a `BanStore` backed by a tempfile and pre-populated with `n`
/// active (non-expired) bans on synthetic IPs.
fn populated_ban_store(n: usize) -> BanStore {
    let tmp = NamedTempFile::new().expect("tempfile");
    let store = BanStore::load(tmp.path().to_str().unwrap());
    for i in 0..n {
        let ip = format!("10.{}.{}.{}", (i >> 16) & 0xff, (i >> 8) & 0xff, i & 0xff);
        store.ban(ip, "seed".into(), 24 * 60 * 60 * 1000);
    }
    store
}

fn bench_ban_store(c: &mut Criterion) {
    let mut group = c.benchmark_group("fraud");
    let store = populated_ban_store(1_000);

    group.bench_function("ban_store_is_banned_hit", |b| {
        b.iter(|| {
            let _ = store.is_banned(black_box("10.0.0.42"));
        });
    });

    group.bench_function("ban_store_is_banned_miss", |b| {
        b.iter(|| {
            let _ = store.is_banned(black_box("203.0.113.99"));
        });
    });

    // Fresh store per measurement to avoid recidivism escalation noise.
    group.bench_function("ban_store_ban_new_ip", |b| {
        let mut counter = 0u32;
        b.iter(|| {
            counter = counter.wrapping_add(1);
            let ip = format!("198.51.100.{}", counter & 0xff);
            store.ban(black_box(ip), "bench".into(), 60_000);
        });
    });

    group.finish();
}

fn bench_fraud_detector(c: &mut Criterion) {
    let mut group = c.benchmark_group("fraud");
    let bans = populated_ban_store(0);
    let detector = FraudDetector::new();

    // Cycle through 10 different IPs so we stay below the ban threshold and
    // measure the *check_and_ban* sliding-window cost rather than the
    // (rare) ban-emission path.
    group.bench_function("fraud_detector_record_login_fail", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let ip = format!("203.0.113.{}", i % 10);
            let _ = detector.record_login_fail(black_box(&ip), black_box(&bans));
        });
    });

    group.finish();
}

fn bench_nonce_store(c: &mut Criterion) {
    let mut group = c.benchmark_group("fraud");
    let store = NonceStore::new();

    group.bench_function("nonce_generate_and_consume", |b| {
        b.iter(|| {
            let n = store.generate().unwrap();
            store.consume(black_box(&n)).unwrap();
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_ban_store,
    bench_fraud_detector,
    bench_nonce_store
);
criterion_main!(benches);
