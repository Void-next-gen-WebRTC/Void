use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::crypto;
use super::models::{Server, ServerChannel};
use super::state::{AppState, ChatEntry};
use crate::auth::middleware::AuthUser;
use crate::errors::ApiError;
use crate::models::UserSummary;
use crate::nonce::NonceStore;

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateServerBody {
    pub name: String,
    pub owner_public_key: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinServerBody {
    pub invite_key: String,
    pub user_public_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedAdminBody {
    pub owner_public_key: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelBody {
    pub name: String,
    pub r#type: String,
    pub owner_public_key: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerResponse {
    pub id: String,
    pub name: String,
    pub owner_public_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_key: Option<String>,
    pub icon: Option<String>,
    pub channels: Vec<ServerChannel>,
    pub members: Vec<String>,
}

impl ServerResponse {
    /// Builds a response, optionally revealing the invite key (owner only).
    fn from_server(s: &Server, reveal_invite_key: bool) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            owner_public_key: s.owner_public_key.clone(),
            invite_key: if reveal_invite_key {
                Some(s.invite_key.clone())
            } else {
                None
            },
            icon: s.icon.clone(),
            channels: s.channels.clone(),
            members: s.members.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolves the caller's Ed25519 public key from the JWT in Authorization header.
fn resolve_caller_public_key(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let auth_user = match AuthUser::from_headers(headers) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("resolve_caller_public_key: JWT failed — {e:?}");
            return None;
        }
    };

    let record = match state.auth_store.users.get(&auth_user.user_id) {
        Some(r) => r,
        None => {
            tracing::warn!(
                "resolve_caller_public_key: user_id={} not found in auth_store",
                auth_user.user_id
            );
            return None;
        }
    };

    let pk = record.public_key.clone();
    if pk.is_none() {
        tracing::warn!(
            "resolve_caller_public_key: user_id={} has no public_key stored",
            auth_user.user_id
        );
    }
    pk
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(create_server).get(list_servers))
        .route("/join-by-invite", post(join_by_invite))
        .route("/:id", get(get_server).delete(delete_server))
        .route("/:id/join", post(join_server))
        .route("/:id/members", get(list_server_members))
        .route("/:id/channels", post(create_channel))
        .route("/:id/channels/:channel_id", delete(delete_channel))
        .route(
            "/:id/channels/:channel_id/messages",
            get(get_channel_messages),
        )
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/servers — creates a server with ownership proof.
async fn create_server(
    State(state): State<Arc<AppState>>,
    Extension(nonces): Extension<NonceStore>,
    Json(body): Json<CreateServerBody>,
) -> Result<Json<ServerResponse>, ApiError> {
    let name = body.name.trim().to_string();
    if name.len() < 2 {
        return Err(ApiError::BadRequest(
            "Name must be at least 2 characters".into(),
        ));
    }

    nonces.consume(&body.nonce)?;

    let message = format!("create:{}:{}", name, body.nonce);
    let valid =
        crypto::verify_signature(&body.owner_public_key, message.as_bytes(), &body.signature)
            .map_err(ApiError::BadRequest)?;

    if !valid {
        return Err(ApiError::Forbidden("Invalid ownership signature".into()));
    }

    let id = Uuid::new_v4().to_string();
    let invite_key = Uuid::new_v4().to_string();
    let owner_pk = body.owner_public_key;

    let server = Server {
        id: id.clone(),
        name,
        owner_public_key: owner_pk.clone(),
        invite_key,
        icon: None,
        channels: vec![
            ServerChannel {
                id: Uuid::new_v4().to_string(),
                name: "general".into(),
                r#type: "text".into(),
            },
            ServerChannel {
                id: Uuid::new_v4().to_string(),
                name: "General".into(),
                r#type: "voice".into(),
            },
        ],
        members: vec![owner_pk.clone()],
    };

    // Owner just created the server — reveal invite key
    let response = ServerResponse::from_server(&server, true);
    state.server_registry.index_member(&owner_pk, &id);
    state.server_registry.servers.insert(id, server);
    state.server_registry.save();

    Ok(Json(response))
}

/// GET /api/servers — lists servers where the authenticated user is a member.
/// Uses the `member_index` for O(1) lookup, falling back to a full scan
/// if the index returns empty (self-healing against stale indexes).
///
/// **Orphaned ownership healing**: when a server's `owner_public_key` no longer
/// maps to any user in the auth store (e.g. after data loss), ownership is
/// automatically transferred to the authenticated caller if they are a member.
async fn list_servers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Vec<ServerResponse>> {
    let caller_pk = resolve_caller_public_key(&state, &headers);

    let servers: Vec<ServerResponse> = match caller_pk {
        Some(ref pk) => {
            // Fast path: secondary index
            let server_ids = state
                .server_registry
                .member_index
                .get(pk)
                .map(|entry| entry.value().clone())
                .unwrap_or_default();

            tracing::debug!(
                "list_servers: pk={}… index returned {} server_id(s)",
                &pk[..pk.len().min(12)],
                server_ids.len()
            );

            let mut results: Vec<ServerResponse> = server_ids
                .iter()
                .filter_map(|sid| state.server_registry.servers.get(sid))
                .map(|kv| {
                    let s = kv.value();
                    let is_owner = caller_pk
                        .as_ref()
                        .is_some_and(|cpk| *cpk == s.owner_public_key);
                    ServerResponse::from_server(s, is_owner)
                })
                .collect();

            // Fallback: full scan if index returned nothing (stale index self-heal)
            if results.is_empty() {
                tracing::debug!("list_servers: index empty — running full scan");
                results = state
                    .server_registry
                    .servers
                    .iter()
                    .filter(|kv| kv.value().members.contains(pk))
                    .map(|kv| {
                        let s = kv.value();
                        // Rebuild index entry for this member
                        state.server_registry.index_member(pk, &s.id);
                        let is_owner = *pk == s.owner_public_key;
                        ServerResponse::from_server(s, is_owner)
                    })
                    .collect();

                if results.is_empty() {
                    tracing::debug!(
                        "list_servers: full scan also empty — no servers contain pk={}…",
                        &pk[..pk.len().min(12)]
                    );
                }
            }

            // Orphaned ownership healing: transfer ownership when the original
            // owner's public key no longer exists in the auth store.
            heal_orphaned_ownership(&state, pk, &mut results);

            results
        }
        None => {
            tracing::debug!("list_servers: no caller_pk resolved — returning empty");
            vec![]
        }
    };

    tracing::debug!("list_servers: returning {} server(s)", servers.len());
    Json(servers)
}

/// Detects servers whose `owner_public_key` is absent from the auth store
/// (orphaned after identity or auth-store data loss) and transfers ownership
/// to the authenticated caller, who must already be a member.
fn heal_orphaned_ownership(state: &AppState, caller_pk: &str, results: &mut [ServerResponse]) {
    let mut healed = false;

    for resp in results.iter_mut() {
        if resp.owner_public_key == caller_pk {
            continue; // Already the owner
        }

        let owner_exists = state
            .auth_store
            .pubkey_index
            .contains_key(&resp.owner_public_key);

        if owner_exists {
            continue; // Owner account still exists — no healing needed
        }

        tracing::warn!(
            "heal_orphaned_ownership: server={} owner_pk={}… has no auth record — transferring to pk={}…",
            resp.id,
            &resp.owner_public_key[..resp.owner_public_key.len().min(12)],
            &caller_pk[..caller_pk.len().min(12)],
        );

        // Update runtime state
        if let Some(mut server) = state.server_registry.servers.get_mut(&resp.id) {
            let old_pk = server.owner_public_key.clone();
            server.owner_public_key = caller_pk.to_string();

            // Also replace old pk in members list if still present
            if let Some(pos) = server.members.iter().position(|m| m == &old_pk) {
                if !server.members.contains(&caller_pk.to_string()) {
                    server.members[pos] = caller_pk.to_string();
                } else {
                    server.members.remove(pos);
                }
            }
        }

        // Update response to reflect new ownership (reveal invite key)
        resp.owner_public_key = caller_pk.to_string();
        if let Some(server) = state.server_registry.servers.get(&resp.id) {
            resp.invite_key = Some(server.invite_key.clone());
        }

        healed = true;
    }

    if healed {
        state.server_registry.save();
    }
}

/// GET /api/servers/:id — returns a single server.
/// Invite key is only revealed when the caller is the owner.
async fn get_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ServerResponse>, ApiError> {
    let caller_pk = resolve_caller_public_key(&state, &headers);

    let server = state
        .server_registry
        .servers
        .get(&id)
        .ok_or_else(|| ApiError::NotFound("Server not found".into()))?;

    let is_owner = caller_pk
        .as_ref()
        .is_some_and(|pk| *pk == server.owner_public_key);

    Ok(Json(ServerResponse::from_server(server.value(), is_owner)))
}

/// GET /api/servers/:id/members — resolves all member public keys into UserSummary profiles.
async fn list_server_members(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<UserSummary>>, ApiError> {
    let server = state
        .server_registry
        .servers
        .get(&id)
        .ok_or_else(|| ApiError::NotFound("Server not found".into()))?;

    let members: Vec<UserSummary> = server
        .members
        .iter()
        .filter_map(|pk| {
            let user_id = state.auth_store.pubkey_index.get(pk)?;
            let record = state.auth_store.users.get(user_id.value())?;
            Some(UserSummary::from(record.value()))
        })
        .collect();

    Ok(Json(members))
}

/// DELETE /api/servers/:id — requires ownership signature.
async fn delete_server(
    State(state): State<Arc<AppState>>,
    Extension(nonces): Extension<NonceStore>,
    Path(id): Path<String>,
    Json(body): Json<SignedAdminBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let server = state
        .server_registry
        .servers
        .get(&id)
        .ok_or_else(|| ApiError::NotFound("Server not found".into()))?;

    if server.owner_public_key != body.owner_public_key {
        return Err(ApiError::Forbidden("Not the server owner".into()));
    }
    drop(server);

    nonces.consume(&body.nonce)?;

    let message = format!("delete:{}:{}", id, body.nonce);
    let valid =
        crypto::verify_signature(&body.owner_public_key, message.as_bytes(), &body.signature)
            .map_err(ApiError::BadRequest)?;

    if !valid {
        return Err(ApiError::Forbidden("Invalid ownership signature".into()));
    }

    state.server_registry.servers.remove(&id);
    state.server_registry.save();

    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// POST /api/servers/join-by-invite — join a server using only the invite key.
async fn join_by_invite(
    State(state): State<Arc<AppState>>,
    Json(body): Json<JoinServerBody>,
) -> Result<Json<ServerResponse>, ApiError> {
    let server_id = state
        .server_registry
        .servers
        .iter()
        .find(|kv| kv.value().invite_key == body.invite_key)
        .map(|kv| kv.key().clone())
        .ok_or_else(|| ApiError::NotFound("Invalid invite key".into()))?;

    let mut server = state
        .server_registry
        .servers
        .get_mut(&server_id)
        .ok_or_else(|| ApiError::NotFound("Server not found".into()))?;

    if !server.members.contains(&body.user_public_key) {
        server.members.push(body.user_public_key.clone());
    }

    let is_owner = server.owner_public_key == body.user_public_key;
    let response = ServerResponse::from_server(&server, is_owner);
    let _pk = body.user_public_key.clone();
    drop(server);
    state.server_registry.index_member(&_pk, &server_id);
    state.server_registry.save();

    Ok(Json(response))
}

/// POST /api/servers/:id/join — join via invite key (legacy, requires server ID).
async fn join_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<JoinServerBody>,
) -> Result<Json<ServerResponse>, ApiError> {
    let mut server = state
        .server_registry
        .servers
        .get_mut(&id)
        .ok_or_else(|| ApiError::NotFound("Server not found".into()))?;

    if server.invite_key != body.invite_key {
        return Err(ApiError::Forbidden("Invalid invite key".into()));
    }

    if !server.members.contains(&body.user_public_key) {
        server.members.push(body.user_public_key.clone());
    }

    let is_owner = server.owner_public_key == body.user_public_key;
    let response = ServerResponse::from_server(&server, is_owner);
    let _pk = body.user_public_key.clone();
    drop(server);
    state.server_registry.index_member(&_pk, &id);
    state.server_registry.save();

    Ok(Json(response))
}

/// POST /api/servers/:id/channels — create channel (owner only).
async fn create_channel(
    State(state): State<Arc<AppState>>,
    Extension(nonces): Extension<NonceStore>,
    Path(id): Path<String>,
    Json(body): Json<CreateChannelBody>,
) -> Result<Json<ServerResponse>, ApiError> {
    let mut server = state
        .server_registry
        .servers
        .get_mut(&id)
        .ok_or_else(|| ApiError::NotFound("Server not found".into()))?;

    if server.owner_public_key != body.owner_public_key {
        return Err(ApiError::Forbidden("Not the server owner".into()));
    }

    nonces.consume(&body.nonce)?;

    let message = format!("create_channel:{}:{}:{}", id, body.name, body.nonce);
    let valid =
        crypto::verify_signature(&body.owner_public_key, message.as_bytes(), &body.signature)
            .map_err(ApiError::BadRequest)?;

    if !valid {
        return Err(ApiError::Forbidden("Invalid ownership signature".into()));
    }

    server.channels.push(ServerChannel {
        id: Uuid::new_v4().to_string(),
        name: body.name.trim().to_string(),
        r#type: body.r#type,
    });

    // Owner action — reveal invite key
    let response = ServerResponse::from_server(&server, true);
    drop(server);
    state.server_registry.save();

    Ok(Json(response))
}

/// DELETE /api/servers/:id/channels/:channel_id — owner only.
async fn delete_channel(
    State(state): State<Arc<AppState>>,
    Extension(nonces): Extension<NonceStore>,
    Path((id, channel_id)): Path<(String, String)>,
    Json(body): Json<SignedAdminBody>,
) -> Result<Json<ServerResponse>, ApiError> {
    let mut server = state
        .server_registry
        .servers
        .get_mut(&id)
        .ok_or_else(|| ApiError::NotFound("Server not found".into()))?;

    if server.owner_public_key != body.owner_public_key {
        return Err(ApiError::Forbidden("Not the server owner".into()));
    }

    nonces.consume(&body.nonce)?;

    let message = format!("delete_channel:{}:{}:{}", id, channel_id, body.nonce);
    let valid =
        crypto::verify_signature(&body.owner_public_key, message.as_bytes(), &body.signature)
            .map_err(ApiError::BadRequest)?;

    if !valid {
        return Err(ApiError::Forbidden("Invalid ownership signature".into()));
    }

    server.channels.retain(|c| c.id != channel_id);

    // Owner action — reveal invite key
    let response = ServerResponse::from_server(&server, true);
    drop(server);
    state.server_registry.save();

    Ok(Json(response))
}

/// GET /api/servers/:id/channels/:channel_id/messages — returns cached chat history.
async fn get_channel_messages(
    State(state): State<Arc<AppState>>,
    Path((_id, channel_id)): Path<(String, String)>,
) -> Json<Vec<ChatEntry>> {
    let history = state.chat_history.read().await;
    let entries = history
        .get(&channel_id)
        .map(|buf| buf.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    Json(entries)
}
