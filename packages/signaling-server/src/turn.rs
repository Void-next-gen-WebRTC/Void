// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Time-limited TURN credential minting, compatible with coturn's
//! `use-auth-secret` / "REST API" mechanism
//! (<https://github.com/coturn/coturn/blob/master/docs/turn-rest-api.md>).
//!
//! Rather than embedding a permanent TURN username/password in the client
//! bundle (trivially extractable, turning the relay into a free open
//! proxy for anyone who bothers to look), the server mints a short-lived
//! credential per authenticated WebSocket session:
//!
//! - `username` = `"{expiry_unix_ts}:{user_id}"`
//! - `credential` = `base64(HMAC-SHA1(shared_secret, username))`
//!
//! coturn is configured with the same `static-auth-secret` and independently
//! recomputes/validates this HMAC — no shared database or RPC needed between
//! the signaling server and coturn.

use base64::{engine::general_purpose, Engine as _};
use std::time::{SystemTime, UNIX_EPOCH};

/// Static configuration for a TURN deployment, read once at startup from
/// environment variables. `None` at the `AppState` level means "no TURN
/// server configured" — clients then fall back to STUN-only, which is
/// known to fail behind symmetric/CGNAT networks (see the ICE failure
/// this module was introduced to fix).
#[derive(Clone)]
pub struct TurnConfig {
    /// One or more `turn:host:port` / `turns:host:port` URLs advertised
    /// to clients, e.g. `["turn:203.0.113.10:3478"]`.
    pub urls: Vec<String>,
    /// Shared secret, identical to coturn's `static-auth-secret` directive.
    pub secret: String,
    /// How long a minted credential remains valid. Kept short (default 2h)
    /// since a fresh one is minted on every `Authenticate` — long-lived
    /// reconnect-free sessions simply get a new credential on reconnect.
    pub ttl_secs: u64,
}

impl TurnConfig {
    /// Builds a config from `TURN_URLS` (comma-separated) and `TURN_SECRET`
    /// environment variables. Returns `None` if either is unset/empty,
    /// which is the expected state for local dev without a TURN server.
    pub fn from_env() -> Option<Self> {
        let urls_raw = std::env::var("TURN_URLS").ok()?;
        let secret = std::env::var("TURN_SECRET").ok()?;
        if urls_raw.trim().is_empty() || secret.trim().is_empty() {
            return None;
        }
        let urls: Vec<String> = urls_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if urls.is_empty() {
            return None;
        }
        let ttl_secs = std::env::var("TURN_CREDENTIAL_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7200);
        Some(Self {
            urls,
            secret,
            ttl_secs,
        })
    }
}

/// Mints a `(username, credential)` pair valid for `config.ttl_secs` from
/// now, scoped to `user_id` (purely informational — coturn does not
/// validate the suffix, only chairs recompute the HMAC over the whole
/// username string).
pub fn mint_credential(config: &TurnConfig, user_id: &str) -> (String, String) {
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + config.ttl_secs;
    let username = format!("{}:{}", expiry, user_id);

    let key = aws_lc_rs::hmac::Key::new(
        aws_lc_rs::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        config.secret.as_bytes(),
    );
    let tag = aws_lc_rs::hmac::sign(&key, username.as_bytes());
    let credential = general_purpose::STANDARD.encode(tag.as_ref());

    (username, credential)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_credential_is_deterministic_for_same_expiry() {
        // Two mints in the same second should be byte-identical (same
        // expiry timestamp, same user, same secret).
        let config = TurnConfig {
            urls: vec!["turn:example.com:3478".to_string()],
            secret: "test-secret".to_string(),
            ttl_secs: 3600,
        };
        let (u1, c1) = mint_credential(&config, "user-1");
        let (u2, c2) = mint_credential(&config, "user-1");
        assert_eq!(u1, u2);
        assert_eq!(c1, c2);
        assert!(u1.ends_with(":user-1"));
    }

    #[test]
    fn mint_credential_differs_per_user() {
        let config = TurnConfig {
            urls: vec!["turn:example.com:3478".to_string()],
            secret: "test-secret".to_string(),
            ttl_secs: 3600,
        };
        let (u1, c1) = mint_credential(&config, "alice");
        let (u2, c2) = mint_credential(&config, "bob");
        assert_ne!(u1, u2);
        assert_ne!(c1, c2);
    }
}
