// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for the adaptive jitter buffer.
//!
//! Run with:
//! ```bash
//! cargo bench -p void-sfu --bench jitter
//! ```

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use webrtc::rtp::header::Header;
use webrtc::rtp::packet::Packet;

use void_sfu::__bench_jitter::JitterBuffer;

/// Builds a synthetic RTP packet with the supplied payload size and the
/// given (sequence, timestamp). The header carries a stable SSRC so the
/// jitter buffer's pruning logic only depends on the timestamp progression.
fn synth_packet(seq: u16, timestamp: u32, payload_size: usize) -> Packet {
    let payload = Bytes::from(vec![0xA5; payload_size]);
    let header = Header {
        version: 2,
        padding: false,
        extension: false,
        marker: false,
        payload_type: 111,
        sequence_number: seq,
        timestamp,
        ssrc: 0xDEAD_BEEF,
        csrc: Vec::new(),
        extension_profile: 0,
        extensions: Vec::new(),
        extensions_padding: 0,
    };
    Packet { header, payload }
}

/// Pre-builds a burst of `count` packets at the given clock cadence so the
/// per-iteration cost is dominated by the jitter buffer, not packet
/// construction.
fn burst(count: usize, payload_size: usize, clock_step: u32) -> Vec<Packet> {
    (0..count)
        .map(|i| synth_packet(i as u16, i as u32 * clock_step, payload_size))
        .collect()
}

fn bench_push(c: &mut Criterion) {
    let mut group = c.benchmark_group("jitter");

    // Opus 20 ms @ 48 kHz: payload ≈ 160 B, timestamp step = 960.
    let opus_pkts = burst(100, 160, 960);
    group.throughput(Throughput::Bytes((100 * 160) as u64));
    group.bench_function(BenchmarkId::new("jitter_push", "opus_20ms_x100"), |b| {
        b.iter(|| {
            let mut buf = JitterBuffer::new(30, 48_000);
            for p in opus_pkts.iter() {
                buf.push(black_box(p.clone()));
            }
        });
    });

    // VP8 1080p: ~1100 B per packet, 90 kHz clock, 30 fps -> step = 3000.
    let vp8_pkts = burst(60, 1100, 3000);
    group.throughput(Throughput::Bytes((60 * 1100) as u64));
    group.bench_function(BenchmarkId::new("jitter_push", "vp8_1080p_x60"), |b| {
        b.iter(|| {
            let mut buf = JitterBuffer::new(50, 90_000);
            for p in vp8_pkts.iter() {
                buf.push(black_box(p.clone()));
            }
        });
    });

    group.finish();
}

fn bench_pop_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("jitter");

    // Pre-fill a buffer with packets older than the playout window so every
    // pop succeeds; we measure the steady-state drain cost.
    let pkts = burst(100, 160, 960);
    group.bench_function("jitter_pop_drain_opus_x100", |b| {
        b.iter_batched(
            || {
                let mut buf = JitterBuffer::new(0, 48_000);
                for p in pkts.iter() {
                    buf.push(p.clone());
                }
                buf
            },
            |mut buf| {
                while let Some(p) = buf.pop() {
                    black_box(p);
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_push, bench_pop_drain);
criterion_main!(benches);
