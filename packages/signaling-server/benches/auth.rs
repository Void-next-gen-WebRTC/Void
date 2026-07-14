// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benchmarks for cryptographic hot paths of the signaling server.
//!
//! Run with:
//! ```bash
//! cargo bench -p signaling-server --bench auth
//! ```
//!
//! Targets the most expensive per-request cost centers:
//! - Argon2id password hashing/verification (kept for legacy migration).
//! - JWT signing and verification (`HS256`).
//! - Ed25519 signature verification — the actual login/register hot path.

use base64::{engine::general_purpose, Engine as _};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;

use signaling_server::auth::jwt::{create_token, decode_token};
use signaling_server::auth::password::{hash_password, verify_password};
use signaling_server::gateway::crypto::verify_signature;

/// Forces the `JWT_SECRET` lazy lock to use the dev fallback so benches do
/// not require any environment configuration.
fn ensure_dev_secret() {
    // SAFETY: benches run single-threaded before any JWT call; setting the
    // env var is idempotent and never observed by production code.
    unsafe {
        std::env::set_var("DEV_MODE", "1");
    }
}

fn bench_argon2(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth");
    // Argon2id is intentionally slow (~hundreds of ms). Keep sample size low.
    group.sample_size(10);

    let password = "correct horse battery staple";
    let stored = hash_password(password).expect("hash");

    group.bench_function("argon2_hash_password", |b| {
        b.iter(|| {
            let _h = hash_password(black_box(password)).unwrap();
        });
    });

    group.bench_function("argon2_verify_password_ok", |b| {
        b.iter(|| {
            assert!(verify_password(black_box(password), black_box(&stored)));
        });
    });

    group.bench_function("argon2_verify_password_ko", |b| {
        b.iter(|| {
            assert!(!verify_password(
                black_box("wrong-password"),
                black_box(&stored)
            ));
        });
    });

    group.finish();
}

fn bench_jwt(c: &mut Criterion) {
    ensure_dev_secret();
    let mut group = c.benchmark_group("auth");

    let user_id = "550e8400-e29b-41d4-a716-446655440000";
    let token = create_token(user_id).expect("create");

    group.bench_function("jwt_create_token", |b| {
        b.iter(|| {
            let _t = create_token(black_box(user_id)).unwrap();
        });
    });

    group.bench_function("jwt_decode_token", |b| {
        b.iter(|| {
            let _c = decode_token(black_box(&token)).unwrap();
        });
    });

    group.finish();
}

fn bench_ed25519(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth");

    // Build a real Ed25519 keypair and sign two representative payloads:
    // a 32-byte challenge (close to the production "register:user:nonce"
    // template) and a 256-byte payload to gauge scaling on larger blobs.
    let mut csprng = OsRng;
    let signing_key = SigningKey::generate(&mut csprng);
    let verifying_key = signing_key.verifying_key();
    let pk_b64 = general_purpose::STANDARD.encode(verifying_key.to_bytes());

    let small: [u8; 32] = [0x42; 32];
    let large: [u8; 256] = [0x77; 256];

    let sig_small = signing_key.sign(&small);
    let sig_large = signing_key.sign(&large);
    let sig_small_b64 = general_purpose::STANDARD.encode(sig_small.to_bytes());
    let sig_large_b64 = general_purpose::STANDARD.encode(sig_large.to_bytes());

    group.throughput(Throughput::Bytes(small.len() as u64));
    group.bench_function("ed25519_verify_signature_32B", |b| {
        b.iter(|| {
            let ok = verify_signature(
                black_box(&pk_b64),
                black_box(&small),
                black_box(&sig_small_b64),
            )
            .unwrap();
            assert!(ok);
        });
    });

    group.throughput(Throughput::Bytes(large.len() as u64));
    group.bench_function("ed25519_verify_signature_256B", |b| {
        b.iter(|| {
            let ok = verify_signature(
                black_box(&pk_b64),
                black_box(&large),
                black_box(&sig_large_b64),
            )
            .unwrap();
            assert!(ok);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_argon2, bench_jwt, bench_ed25519);
criterion_main!(benches);
