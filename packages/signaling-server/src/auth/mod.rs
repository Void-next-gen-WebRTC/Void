pub mod jwt;
pub mod middleware;
pub mod password;

use std::net::SocketAddr;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Extension, Router};
use uuid::Uuid;

use crate::errors::ApiError;
use crate::fraud::FraudState;
use crate::gateway::crypto;
use crate::gateway::registry::ServerRegistry;
use crate::models::*;
use crate::negotiate::{accepts_proto, decode_body, negotiate, negotiate_list, Negotiated};
use crate::nonce::NonceStore;
use crate::store::{Store, UserRecord};
use middleware::AuthUser;

/// Builds the `/api/auth` sub-router.
pub fn router() -> Router<Store> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/me", get(me).patch(update_profile))
        .route("/users/search", get(search_users))
}

/// POST /api/auth/register
/// Password-free — auth relies solely on Ed25519 nonce challenge-response.
async fn register(
    State(store): State<Store>,
    Extension(nonces): Extension<NonceStore>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let body: RegisterBody = decode_body(&headers, &body)?;

    let username = body.username.trim().to_lowercase();
    if username.len() < 2 {
        return Err(ApiError::BadRequest(
            "Username must be at least 2 characters".into(),
        ));
    }

    let pk = &body.public_key;
    if pk.is_empty() {
        return Err(ApiError::BadRequest("public_key is required".into()));
    }

    // Nonce challenge: consume → verify Ed25519 signature
    nonces.consume(&body.nonce)?;
    let challenge = format!("register:{}:{}", username, body.nonce);
    let valid = crypto::verify_signature(pk, challenge.as_bytes(), &body.signature)
        .map_err(ApiError::BadRequest)?;
    if !valid {
        return Err(ApiError::Forbidden("Invalid identity signature".into()));
    }

    // Username is a display name — uniqueness is on the public key only
    if store.pubkey_index.contains_key(pk) {
        return Err(ApiError::Conflict("Public key already registered".into()));
    }

    let id = Uuid::new_v4().to_string();
    let now_ms = epoch_ms();

    let record = UserRecord {
        id: id.clone(),
        username: username.clone(),
        display_name: body.display_name.trim().to_string(),
        password_hash: None,
        avatar: None,
        public_key: Some(pk.to_string()),
        created_at_ms: now_ms,
    };

    let profile = UserProfile::from(&record);
    store.username_index.insert(username, id.clone());
    store.pubkey_index.insert(pk.to_string(), id.clone());
    store.users.insert(id.clone(), record);
    store.mark_dirty();

    let token = jwt::create_token(&id).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(negotiate(
        AuthResponse {
            token,
            user: Some(profile),
        },
        proto,
    ))
}

/// POST /api/auth/login
/// Password-free — the server identifies the user by `public_key` via
/// `pubkey_index` and verifies ownership through Ed25519 nonce challenge.
async fn login(
    State(store): State<Store>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Extension(fraud): Extension<FraudState>,
    Extension(nonces): Extension<NonceStore>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let body: LoginBody = decode_body(&headers, &body)?;
    let ip = addr.ip().to_string();

    if fraud.bans.is_banned(&ip) {
        return Err(ApiError::Forbidden("Access denied".into()));
    }

    let pk = &body.public_key;
    if pk.is_empty() {
        return Err(ApiError::BadRequest("public_key is required".into()));
    }

    // Nonce challenge: consume → verify Ed25519 signature
    nonces.consume(&body.nonce)?;
    let challenge = format!("login:{}:{}", pk, body.nonce);
    let valid = crypto::verify_signature(pk, challenge.as_bytes(), &body.signature)
        .map_err(ApiError::BadRequest)?;
    if !valid {
        return Err(ApiError::Forbidden("Invalid identity signature".into()));
    }

    let user_id = store
        .pubkey_index
        .get(pk.as_str())
        .map(|r| r.value().clone())
        .ok_or_else(|| {
            fraud.detector.record_login_fail(&ip, &fraud.bans);
            ApiError::Unauthorized("Unknown public key — register first".into())
        })?;

    let profile = store
        .users
        .get(&user_id)
        .map(|r| UserProfile::from(r.value()))
        .ok_or_else(|| ApiError::Internal("User disappeared".into()))?;
    let token = jwt::create_token(&user_id).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(negotiate(
        AuthResponse {
            token,
            user: Some(profile),
        },
        proto,
    ))
}

/// GET /api/auth/me
async fn me(State(store): State<Store>, headers: HeaderMap) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let record = store
        .users
        .get(&auth.user_id)
        .ok_or_else(|| ApiError::NotFound("User not found".into()))?;
    Ok(negotiate(UserProfile::from(record.value()), proto))
}

/// PATCH /api/auth/me
async fn update_profile(
    State(store): State<Store>,
    Extension(registry): Extension<ServerRegistry>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let body: UpdateProfileBody = decode_body(&headers, &body)?;

    let mut record = store
        .users
        .get_mut(&auth.user_id)
        .ok_or_else(|| ApiError::NotFound("User not found".into()))?;

    if let Some(name) = body.display_name {
        record.display_name = name.trim().to_string();
    }
    if let Some(avatar) = body.avatar {
        record.avatar = Some(avatar);
    }

    // Sync Ed25519 public key when the client provides a newer one
    if let Some(ref new_pk) = body.public_key {
        if record.public_key.as_ref() != Some(new_pk) {
            let old_pk = record.public_key.clone();
            let new_pk_owned = new_pk.clone();

            tracing::info!(
                "update_profile: pk sync for user={} old={:?} new={}…",
                record.username,
                old_pk.as_deref().map(|s| &s[..s.len().min(12)]),
                &new_pk_owned[..new_pk_owned.len().min(12)]
            );

            if let Some(ref opk) = old_pk {
                store.pubkey_index.remove(opk);
            }
            store
                .pubkey_index
                .insert(new_pk_owned.clone(), auth.user_id.clone());
            record.public_key = Some(new_pk_owned.clone());
            let profile = UserProfile::from(&*record);
            drop(record);
            store.mark_dirty();

            if let Some(ref opk) = old_pk {
                migrate_server_memberships(&registry, opk, &new_pk_owned);
            }

            return Ok(negotiate(profile, proto));
        }
    }

    let profile = UserProfile::from(&*record);
    store.mark_dirty();
    Ok(negotiate(profile, proto))
}

/// GET /api/auth/users/search?q=<query>
/// Searches by display_name, username, tag (pseudo#XXXX) or public_key.
async fn search_users(
    State(store): State<Store>,
    headers: HeaderMap,
    Query(params): Query<SearchQuery>,
) -> Result<Negotiated, ApiError> {
    let proto = accepts_proto(&headers);
    let auth = AuthUser::from_headers(&headers)?;
    let q = params.q.trim().to_lowercase();
    tracing::debug!("search_users: raw_q={:?} parsed_q={:?}", params.q, q);
    if q.is_empty() {
        return Ok(negotiate_list(
            vec![],
            |items| UserSummaryList { items },
            proto,
        ));
    }

    // Parse tag format: "pseudo#XXXX"
    let (tag_name, tag_suffix) = if let Some(pos) = q.find('#') {
        let name = q[..pos].to_string();
        let suffix = q[pos + 1..].to_uppercase();
        if !name.is_empty() && !suffix.is_empty() {
            (Some(name), Some(suffix))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let results: Vec<UserSummary> = store
        .users
        .iter()
        .filter(|r| {
            let u = r.value();
            if u.id == auth.user_id {
                return false;
            }

            // Tag mode (`pseudo#XXXX`): STRICT match — both the name part
            // and the public-key suffix must match. No name-only fallback,
            // otherwise unrelated homonyms leak into the results.
            if let (Some(name), Some(suffix)) = (&tag_name, &tag_suffix) {
                let name_matches =
                    u.display_name.to_lowercase().contains(name) || u.username.contains(name);
                if !name_matches {
                    return false;
                }
                if let Some(ref pk) = u.public_key {
                    let pk_suffix: String = pk
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                        .to_uppercase();
                    return pk_suffix.starts_with(suffix.as_str());
                }
                return false;
            }

            // Match by public_key substring
            if let Some(ref pk) = u.public_key {
                if pk.to_lowercase().contains(&q) {
                    return true;
                }
            }

            // Match by display_name or username (plain text)
            u.display_name.to_lowercase().contains(&q) || u.username.contains(&q)
        })
        .take(20)
        .map(|r| UserSummary::from(r.value()))
        .collect();

    tracing::debug!("search_users: {} result(s) for q={:?}", results.len(), q);
    Ok(negotiate_list(
        results,
        |items| UserSummaryList { items },
        proto,
    ))
}

/// Replaces `old_pk` with `new_pk` in every server's member list and in
/// the `member_index` secondary index.  Called when a login syncs a new
/// Ed25519 public key.
///
/// Uses the secondary `member_index` for fast lookup, then falls back to a
/// full scan of all servers when the index is stale or empty.  After
/// migration the registry is flushed **synchronously** to prevent data loss
/// if the process restarts before the debounced flusher runs.
fn migrate_server_memberships(registry: &ServerRegistry, old_pk: &str, new_pk: &str) {
    // Fast path: move server IDs from old pk → new pk via member_index
    let mut server_ids: Vec<String> = registry
        .member_index
        .remove(old_pk)
        .map(|(_, ids)| ids)
        .unwrap_or_default();

    // Fallback: full scan when index is empty (self-healing for stale indexes)
    if server_ids.is_empty() {
        tracing::warn!("member_index empty for old_pk during migration — running full scan");
        server_ids = registry
            .servers
            .iter()
            .filter(|kv| kv.value().members.contains(&old_pk.to_string()))
            .map(|kv| kv.key().clone())
            .collect();
    }

    if server_ids.is_empty() {
        tracing::info!("No server memberships found for old_pk — nothing to migrate");
        return;
    }

    for sid in &server_ids {
        if let Some(mut server) = registry.servers.get_mut(sid) {
            if let Some(pos) = server.members.iter().position(|m| m == old_pk) {
                server.members[pos] = new_pk.to_string();
            }
            // Also migrate ownership when the owner's key changes
            if server.owner_public_key == old_pk {
                tracing::info!("Migrating owner_public_key for server={}", sid);
                server.owner_public_key = new_pk.to_string();
            }
        }
        registry.index_member(new_pk, sid);
    }

    // Synchronous flush: critical migration data must survive a restart
    registry.flush_sync();
    tracing::info!(
        "Migrated {} server memberships from old pk to new pk",
        server_ids.len()
    );
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
