use std::collections::VecDeque;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use super::store::BanStore;
use super::{BANNED_IPS_TOTAL, FRAUD_ATTEMPTS};

/// Default ban duration: 24 hours in milliseconds.
const BAN_DURATION_MS: i64 = 24 * 60 * 60 * 1000;

/// Sliding-window thresholds (count / window).
const LOGIN_FAIL_LIMIT: usize = 10;
const LOGIN_FAIL_WINDOW: Duration = Duration::from_secs(60);

const INVALID_TOKEN_LIMIT: usize = 5;
const INVALID_TOKEN_WINDOW: Duration = Duration::from_secs(60);

const WS_FLOOD_LIMIT: usize = 20;
const WS_FLOOD_WINDOW: Duration = Duration::from_secs(60);

/// Heuristic-based fraud detector using sliding windows per IP.
pub struct FraudDetector {
    login_fails: DashMap<String, VecDeque<Instant>>,
    invalid_tokens: DashMap<String, VecDeque<Instant>>,
    ws_connects: DashMap<String, VecDeque<Instant>>,
}

impl FraudDetector {
    pub fn new() -> Self {
        Self {
            login_fails: DashMap::new(),
            invalid_tokens: DashMap::new(),
            ws_connects: DashMap::new(),
        }
    }
}

impl Default for FraudDetector {
    fn default() -> Self {
        Self::new()
    }
}

    /// Records a failed login attempt. Returns `true` if the IP should be banned.
    pub fn record_login_fail(&self, ip: &str, bans: &BanStore) -> bool {
        FRAUD_ATTEMPTS
            .with_label_values(&["login_bruteforce"])
            .inc();
        self.check_and_ban(
            &self.login_fails,
            ip,
            LOGIN_FAIL_LIMIT,
            LOGIN_FAIL_WINDOW,
            "login_bruteforce",
            bans,
        )
    }

    /// Records an invalid JWT/token attempt. Returns `true` if banned.
    pub fn record_invalid_token(&self, ip: &str, bans: &BanStore) -> bool {
        FRAUD_ATTEMPTS.with_label_values(&["invalid_token"]).inc();
        self.check_and_ban(
            &self.invalid_tokens,
            ip,
            INVALID_TOKEN_LIMIT,
            INVALID_TOKEN_WINDOW,
            "invalid_token",
            bans,
        )
    }

    /// Records a WebSocket connection. Returns `true` if banned.
    pub fn record_ws_connect(&self, ip: &str, bans: &BanStore) -> bool {
        FRAUD_ATTEMPTS.with_label_values(&["ws_flood"]).inc();
        self.check_and_ban(
            &self.ws_connects,
            ip,
            WS_FLOOD_LIMIT,
            WS_FLOOD_WINDOW,
            "ws_flood",
            bans,
        )
    }

    /// Generic sliding window check → ban if threshold exceeded.
    fn check_and_ban(
        &self,
        map: &DashMap<String, VecDeque<Instant>>,
        ip: &str,
        limit: usize,
        window: Duration,
        reason: &str,
        bans: &BanStore,
    ) -> bool {
        let now = Instant::now();
        let mut entry = map.entry(ip.to_string()).or_default();
        let deque = entry.value_mut();

        // Purge events outside the window
        while deque
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            deque.pop_front();
        }
        deque.push_back(now);

        if deque.len() >= limit {
            deque.clear();
            bans.ban(ip.to_string(), reason.to_string(), BAN_DURATION_MS);
            BANNED_IPS_TOTAL.inc();
            tracing::warn!("IP {ip} banned for {reason}");
            return true;
        }
        false
    }

    /// Periodic cleanup of stale entries (call every ~60s).
    pub fn cleanup(&self) {
        let cutoff = Instant::now() - Duration::from_secs(120);
        for map in [&self.login_fails, &self.invalid_tokens, &self.ws_connects] {
            map.retain(|_, deque| {
                deque.retain(|t| *t > cutoff);
                !deque.is_empty()
            });
        }
    }
}

/// Spawns a background task that cleans stale detector entries every 60s.
pub fn spawn_cleanup(detector: std::sync::Arc<FraudDetector>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            detector.cleanup();
        }
    });
}
