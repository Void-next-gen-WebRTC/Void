// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for the protobuf / JSON codec hot paths.
//!
//! Run with:
//! ```bash
//! cargo bench -p signaling-server --bench codec
//! ```

use axum::body::Bytes;
use axum::http::{header, HeaderMap};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use prost::Message;

use signaling_server::models::{
    AuthResponse, LoginBody, PendingRequest, PendingRequestList, UserProfile, UserSummary,
    UserSummaryList,
};
use signaling_server::negotiate::decode_body;
use signaling_server::gateway::broadcast::serialize_message;
use signaling_server::gateway::models::ServerMessage;

fn sample_login_body() -> LoginBody {
    LoginBody {
        public_key: "MCowBQYDK2VwAyEAabcdefghijklmnopqrstuvwxyz0123456789AAAA".into(),
        nonce: "550e8400-e29b-41d4-a716-446655440000".into(),
        signature: "z".repeat(88),
    }
}

fn sample_user_profile(i: usize) -> UserProfile {
    UserProfile {
        id: format!("550e8400-e29b-41d4-a716-{:012x}", i),
        username: format!("user{i}"),
        display_name: format!("User Number {i}"),
        avatar: Some("https://cdn.example.com/avatar.png".into()),
        public_key: Some(format!("pubkey{:0>56}", i)),
        created_at_ms: 1_700_000_000_000 + i as i64,
    }
}

fn sample_user_summary(i: usize) -> UserSummary {
    UserSummary {
        id: format!("550e8400-e29b-41d4-a716-{:012x}", i),
        username: format!("user{i}"),
        display_name: format!("User #{i}"),
        avatar: Some("https://cdn.example.com/avatar.png".into()),
        public_key: Some(format!("pubkey{:0>56}", i)),
    }
}

fn sample_pending(i: usize) -> PendingRequest {
    PendingRequest {
        id: format!("req-{i:08}"),
        from: Some(sample_user_summary(i)),
        created_at_ms: 1_700_000_000_000 + i as i64,
    }
}

fn proto_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        "application/x-protobuf".parse().unwrap(),
    );
    h
}

fn json_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    h
}

fn bench_login_body(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    let body = sample_login_body();
    let bytes = body.encode_to_vec();
    let json = serde_json::to_vec(&serde_json::json!({
        "publicKey": body.public_key,
        "nonce": body.nonce,
        "signature": body.signature,
    }))
    .unwrap();

    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("proto_encode_login_body", |b| {
        b.iter(|| black_box(body.encode_to_vec()));
    });

    group.bench_function("proto_decode_login_body", |b| {
        b.iter(|| {
            let _ = LoginBody::decode(black_box(bytes.as_slice())).unwrap();
        });
    });

    let proto_h = proto_headers();
    let json_h = json_headers();
    let proto_body = Bytes::from(bytes.clone());
    let json_body = Bytes::from(json);

    group.bench_function("negotiate_decode_body_proto", |b| {
        b.iter(|| {
            let _: LoginBody = decode_body(black_box(&proto_h), black_box(&proto_body)).unwrap();
        });
    });

    group.bench_function("negotiate_decode_body_json", |b| {
        b.iter(|| {
            let _: LoginBody = decode_body(black_box(&json_h), black_box(&json_body)).unwrap();
        });
    });

    group.finish();
}

fn bench_auth_response(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    let resp = AuthResponse {
        token: "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.".repeat(3),
        user: Some(sample_user_profile(42)),
    };
    let bytes = resp.encode_to_vec();

    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("proto_encode_auth_response", |b| {
        b.iter(|| black_box(resp.encode_to_vec()));
    });
    group.bench_function("proto_decode_auth_response", |b| {
        b.iter(|| {
            let _ = AuthResponse::decode(black_box(bytes.as_slice())).unwrap();
        });
    });
    group.finish();
}

fn bench_user_summary_list(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    for &n in &[10usize, 100, 1000] {
        let list = UserSummaryList {
            items: (0..n).map(sample_user_summary).collect(),
        };
        let bytes = list.encode_to_vec();
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("proto_encode_user_summary_list", n),
            &list,
            |b, l| b.iter(|| black_box(l.encode_to_vec())),
        );

        group.bench_with_input(
            BenchmarkId::new("proto_decode_user_summary_list", n),
            &bytes,
            |b, buf| {
                b.iter(|| {
                    let _ = UserSummaryList::decode(black_box(buf.as_slice())).unwrap();
                });
            },
        );
    }
    group.finish();
}

fn bench_pending_request_list(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    for &n in &[10usize, 100] {
        let list = PendingRequestList {
            items: (0..n).map(sample_pending).collect(),
        };
        let bytes = list.encode_to_vec();
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("proto_encode_pending_request_list", n),
            &list,
            |b, l| b.iter(|| black_box(l.encode_to_vec())),
        );

        group.bench_with_input(
            BenchmarkId::new("proto_decode_pending_request_list", n),
            &bytes,
            |b, buf| {
                b.iter(|| {
                    let _ = PendingRequestList::decode(black_box(buf.as_slice())).unwrap();
                });
            },
        );
    }
    group.finish();
}

fn bench_server_message(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    let msg = ServerMessage::Chat {
        channel_id: "chan-12345".into(),
        from: "550e8400-e29b-41d4-a716-446655440000".into(),
        username: "alice".into(),
        message: "Hello, world! This is a representative chat payload.".into(),
        timestamp: 1_700_000_000_000,
    };
    group.bench_function("serialize_server_message_chat_json", |b| {
        b.iter(|| {
            let _ = serialize_message(black_box(&msg)).unwrap();
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_login_body,
    bench_auth_response,
    bench_user_summary_list,
    bench_pending_request_list,
    bench_server_message,
);
criterion_main!(benches);
