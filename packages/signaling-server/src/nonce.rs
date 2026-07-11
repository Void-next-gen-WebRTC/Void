/// Nonce-based anti-replay store.
///
/// Each nonce is a UUID v4 string stored with its creation instant.
/// Nonces are single-use: consumed atomically on first verification.
/// A background task prunes expired entries every 30 seconds.
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use axum::{Extension, Json};
use dashmap::DashMap;
use serde::Serialize;
use uuid::Uuid;

use crate::errors::ApiError;

/// Time-to-live for a nonce before it expires unused.
const NONCE_TTL: Duration = Duration::from_secs(120);

/// Cleanup interval for the background pruning task.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

/// Maximum pending nonces before the store refuses new ones (DoS guard).
const MAX_PENDING: usize = 10_000;

/// In-memory store mapping nonce strings to their creation timestamp.
#[derive(Clone)]
pub struct NonceStore {
    inner: Arc<DashMap<String, Instant>>,
}

#[derive(Serialize)]
pub struct NonceResponse {
    pub nonce: String,
}

impl NonceStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Generates a fresh nonce, stores it, and returns the string.
    pub fn generate(&self) -> Result<String, ApiError> {
        if self.inner.len() >= MAX_PENDING {
            return Err(ApiError::TooManyRequests(
                "Too many pending nonces — try again later".into(),
            ));
        }
        let nonce = Uuid::new_v4().to_string();
        self.inner.insert(nonce.clone(), Instant::now());
        Ok(nonce)
    }

    /// Atomically consumes a nonce: removes it and checks TTL.
    /// Returns `Ok(())` on success, or `ApiError::Forbidden` when the
    /// nonce is missing, already consumed, or expired.
    pub fn consume(&self, nonce: &str) -> Result<(), ApiError> {
        let (_, created_at) = self
            .inner
            .remove(nonce)
            .ok_or_else(|| ApiError::Forbidden("Invalid or already used nonce".into()))?;

        if created_at.elapsed() > NONCE_TTL {
            return Err(ApiError::Forbidden("Nonce expired".into()));
        }
        Ok(())
    }

    /// Removes all entries older than `NONCE_TTL`.
    fn prune(&self) {
        let before = self.inner.len();
        self.inner
            .retain(|_, created_at| created_at.elapsed() <= NONCE_TTL);
        let removed = before - self.inner.len();
        if removed > 0 {
            tracing::debug!("NonceStore: pruned {} expired nonce(s)", removed);
        }
    }
}

impl Default for NonceStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a periodic background task that prunes expired nonces.
pub fn spawn_cleanup(store: NonceStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            store.prune();
        }
    });
}

/// GET /api/auth/nonce — issues a single-use nonce for challenge-response signing.
pub async fn get_nonce(
    Extension(store): Extension<NonceStore>,
    _headers: HeaderMap,
) -> Result<Json<NonceResponse>, ApiError> {
    let nonce = store.generate()?;
    Ok(Json(NonceResponse { nonce }))
}
