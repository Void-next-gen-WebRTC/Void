pub mod detector;
pub mod store;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use self::store::BanStore;

/// Shared state for the fraud subsystem.
#[derive(Clone)]
pub struct FraudState {
    pub bans: BanStore,
    pub detector: Arc<detector::FraudDetector>,
}

/// Axum middleware that rejects requests from banned IPs.
pub async fn ip_guard(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::extract::Extension(state): axum::extract::Extension<FraudState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = addr.ip().to_string();

    if state.bans.is_banned(&ip) {
        BLOCKED_REQUESTS.inc();
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Access denied" })),
        )
            .into_response();
    }

    next.run(req).await
}

// ---------------------------------------------------------------------------
// Prometheus counters
// ---------------------------------------------------------------------------

use once_cell::sync::Lazy;
use prometheus::{register_int_counter, register_int_counter_vec, IntCounter, IntCounterVec};

pub static BANNED_IPS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!("fraud_ip_banned_total", "Total IPs banned for fraud").unwrap()
});

pub static PERMANENT_BANS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "fraud_permanent_bans_total",
        "IPs escalated to permanent ban via recidivism"
    )
    .unwrap()
});

pub static FINGERPRINT_BANS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "fraud_fingerprint_bans_total",
        "IPs banned via shared device fingerprint abuse"
    )
    .unwrap()
});

pub static BLOCKED_REQUESTS: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "fraud_blocked_requests_total",
        "Requests blocked by IP ban middleware"
    )
    .unwrap()
});

pub static FRAUD_ATTEMPTS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "fraud_attempts_total",
        "Fraud detection events by type",
        &["type"]
    )
    .unwrap()
});
