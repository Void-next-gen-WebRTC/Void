use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::ConnectInfo;
use axum::response::IntoResponse;
use axum::Extension;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use void_sfu::{PeerId, RoomId, SignalSink};

use super::adapter::WsPeerSink;
use super::broadcast::{broadcast_to_channel, remove_peer, serialize_message};
use super::handler_helpers::{handle_authenticate, handle_dm_send};
use super::models::{ClientMessage, PeerInfo, ServerMessage};
use super::rpc as ws_rpc;
use super::state::{AppState, ChatEntry, PeerSession, CHAT_HISTORY_CAP, WS_CHANNEL_CAPACITY};
use super::subscriptions::{push_to_channel_subscribers, push_to_server_subscribers};
use crate::fraud::FraudState;
use crate::metrics::WS_QUEUE_DROPPED;

/// Axum WebSocket upgrade handler.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    fraud: Option<Extension<FraudState>>,
) -> impl IntoResponse {
    let ip = addr.ip().to_string();
    let fraud_state = fraud.map(|Extension(f)| f);
    ws.on_upgrade(move |socket| handle_socket(socket, state, ip, fraud_state))
}

/// Manages a single peer's WebSocket lifecycle.
async fn handle_socket(
    socket: WebSocket,
    state: Arc<AppState>,
    ip: String,
    fraud_state: Option<FraudState>,
) {
    let (mut socket_sender, mut socket_receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel::<String>(WS_CHANNEL_CAPACITY);

    // Voice user_id (set by `Join`); identifies the peer in the SFU.
    let mut current_user_id: Option<String> = None;
    // Authenticated user_id (set by `Authenticate`); required for any RPC.
    let mut auth_user_id: Option<String> = None;
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if socket_sender.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = socket_receiver.next().await {
        let text = match message {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        let msg = match serde_json::from_str::<ClientMessage>(&text) {
            Ok(m) => m,
            Err(_) => continue,
        };

        match msg {
            ClientMessage::Join {
                channel_id,
                user_id,
                username,
                fingerprint,
            } => {
                if let (Some(fp), Some(fs)) = (&fingerprint, &fraud_state) {
                    fs.bans.record_fingerprint(fp, &ip);
                }
                if let Err(e) = handle_join(
                    &state,
                    &tx,
                    &mut current_user_id,
                    channel_id,
                    user_id,
                    username,
                )
                .await
                {
                    warn!("join failed: {:?}", e);
                }
            }

            ClientMessage::Offer { sdp } => {
                if let Some(uid) = current_user_id.as_deref() {
                    let pid = PeerId::from(uid);
                    let sdp_str = extract_sdp(&sdp);
                    if let Err(e) = state.sfu.handle_offer(&pid, sdp_str).await {
                        warn!("sfu.handle_offer({}): {:?}", uid, e);
                    }
                }
            }

            ClientMessage::Answer { sdp } => {
                if let Some(uid) = current_user_id.as_deref() {
                    let pid = PeerId::from(uid);
                    let sdp_str = extract_sdp(&sdp);
                    if let Err(e) = state.sfu.handle_answer(&pid, sdp_str).await {
                        debug!("sfu.handle_answer({}): {:?}", uid, e);
                    }
                }
            }

            ClientMessage::Ice { candidate } => {
                if let Some(uid) = current_user_id.as_deref() {
                    let pid = PeerId::from(uid);
                    match parse_ice_candidate(&candidate) {
                        Some(ice) => {
                            if let Err(e) = state.sfu.handle_ice(&pid, ice).await {
                                debug!("sfu.handle_ice({}): {:?}", uid, e);
                            }
                        }
                        None => {
                            debug!("sfu.handle_ice({}): malformed ICE payload", uid);
                        }
                    }
                }
            }

            ClientMessage::MediaState {
                channel_id,
                user_id,
                is_muted,
                is_deafened,
            } => {
                let uid_for_exclude = user_id.clone();
                if let Some(peer) = state.peers.write().await.get_mut(&user_id) {
                    peer.is_muted = is_muted;
                    peer.is_deafened = is_deafened;
                }
                broadcast_to_channel(
                    &state,
                    &channel_id,
                    &ServerMessage::PeerState {
                        channel_id: channel_id.clone(),
                        user_id,
                        is_muted,
                        is_deafened,
                    },
                    Some(uid_for_exclude.as_str()),
                )
                .await;
            }

            ClientMessage::Chat {
                channel_id,
                from,
                username,
                message,
                timestamp,
            } => {
                let entry = ChatEntry {
                    id: format!("{}-{}", from, timestamp),
                    channel_id: channel_id.clone(),
                    from: from.clone(),
                    username: username.clone(),
                    message: message.clone(),
                    timestamp,
                };

                {
                    let mut history = state.chat_history.write().await;
                    let buf = history.entry(channel_id.clone()).or_insert_with(|| {
                        std::collections::VecDeque::with_capacity(CHAT_HISTORY_CAP)
                    });
                    if buf.len() >= CHAT_HISTORY_CAP {
                        buf.pop_front();
                    }
                    buf.push_back(entry);
                }

                let payload = ServerMessage::Chat {
                    channel_id: channel_id.clone(),
                    from,
                    username,
                    message,
                    timestamp,
                };
                // Voice-room broadcast (legacy path) + text-channel subscribers.
                broadcast_to_channel(&state, &channel_id, &payload, None).await;
                push_to_channel_subscribers(&state, &channel_id, &payload, None).await;
            }

            ClientMessage::Leave { .. } => {
                if let Some(uid) = current_user_id.take() {
                    remove_peer(&state, &uid).await;
                }
            }

            ClientMessage::Authenticate { token } => {
                handle_authenticate(&state, &tx, &mut auth_user_id, token).await;
            }

            ClientMessage::SubscribeChannel { channel_id } => {
                if let Some(uid) = auth_user_id.as_deref() {
                    state.subscriptions.subscribe_channel(&channel_id, uid);
                }
            }

            ClientMessage::UnsubscribeChannel { channel_id } => {
                if let Some(uid) = auth_user_id.as_deref() {
                    state.subscriptions.unsubscribe_channel(&channel_id, uid);
                }
            }

            ClientMessage::SubscribeServer { server_id } => {
                if let Some(uid) = auth_user_id.as_deref() {
                    state.subscriptions.subscribe_server(&server_id, uid);
                    // Push self-presence so other subscribers see the new arrival.
                    push_to_server_subscribers(
                        &state,
                        &server_id,
                        &ServerMessage::ServerMemberPresence {
                            server_id: server_id.clone(),
                            user_id: uid.to_string(),
                            online: true,
                        },
                        Some(uid),
                    )
                    .await;
                }
            }

            ClientMessage::UnsubscribeServer { server_id } => {
                if let Some(uid) = auth_user_id.as_deref() {
                    state.subscriptions.unsubscribe_server(&server_id, uid);
                }
            }

            ClientMessage::Rpc {
                request_id,
                method,
                params,
            } => {
                ws_rpc::dispatch(
                    &state,
                    auth_user_id.as_deref(),
                    request_id,
                    method,
                    params,
                    &tx,
                )
                .await;
            }

            ClientMessage::DmSend {
                to_user_id,
                message,
                client_msg_id,
            } => {
                handle_dm_send(
                    &state,
                    &tx,
                    auth_user_id.as_deref(),
                    to_user_id,
                    message,
                    client_msg_id,
                )
                .await;
            }
        }
    }

    if let Some(user_id) = current_user_id {
        remove_peer(&state, &user_id).await;
    }
    if let Some(uid) = auth_user_id.take() {
        // Drop the auth-keyed binding first so no late push can race with
        // shutdown (notify_user becomes a no-op for `uid`).
        state.subscriptions.unbind_user(&uid);
        let servers = state.subscriptions.drop_user(&uid);
        for server_id in servers {
            let sid_clone = server_id.clone();
            push_to_server_subscribers(
                &state,
                &server_id,
                &ServerMessage::ServerMemberPresence {
                    server_id: sid_clone,
                    user_id: uid.clone(),
                    online: false,
                },
                Some(&uid),
            )
            .await;
        }
    }
    send_task.abort();
}

/// Processes a Join message: registers the peer in the SFU, persists host-side
/// metadata, and emits the snapshot `Joined` message back to the joiner.
async fn handle_join(
    state: &Arc<AppState>,
    tx: &mpsc::Sender<String>,
    current_user_id: &mut Option<String>,
    channel_id: String,
    user_id: String,
    username: String,
) -> Result<(), void_sfu::SfuError> {
    if let Some(old_id) = current_user_id.take() {
        if old_id != user_id {
            remove_peer(state, &old_id).await;
        }
    }

    let peer_id = PeerId::from(user_id.as_str());

    if state.sfu.peer_room(&peer_id).is_none() && !sfu_knows_peer(state, &peer_id) {
        let sink: Arc<dyn SignalSink> = Arc::new(WsPeerSink::new(tx.clone()));
        if let Err(e) = state.sfu.add_peer(peer_id.clone(), sink) {
            debug!("add_peer race: {:?} — recovering", e);
            let _ = state.sfu.remove_peer(&peer_id).await;
            let sink: Arc<dyn SignalSink> = Arc::new(WsPeerSink::new(tx.clone()));
            state.sfu.add_peer(peer_id.clone(), sink)?;
        }
    }

    state.peers.write().await.insert(
        user_id.clone(),
        PeerSession {
            user_id: user_id.clone(),
            username: username.clone(),
            channel_id: channel_id.clone(),
            tx: tx.clone(),
            is_muted: false,
            is_deafened: false,
        },
    );

    let snapshot = state
        .sfu
        .join_room(&peer_id, RoomId::from(channel_id.as_str()))
        .await?;

    let existing_peers: Vec<PeerInfo> = {
        let peers = state.peers.read().await;
        snapshot
            .existing_peers
            .iter()
            .filter_map(|p| {
                peers.get(p.peer_id.as_str()).map(|host| PeerInfo {
                    user_id: host.user_id.clone(),
                    username: host.username.clone(),
                    is_muted: host.is_muted,
                    is_deafened: host.is_deafened,
                })
            })
            .collect()
    };

    if let Some(payload) = serialize_message(&ServerMessage::Joined {
        channel_id: channel_id.clone(),
        peers: existing_peers,
        started_at: snapshot.started_at_ms,
    }) {
        if tx.try_send(payload).is_err() {
            WS_QUEUE_DROPPED.inc();
        }
    }

    *current_user_id = Some(user_id);
    Ok(())
}

fn sfu_knows_peer(state: &Arc<AppState>, peer_id: &PeerId) -> bool {
    state.sfu.peer_room(peer_id).is_some()
}

/// Coerces an inbound SDP payload (`serde_json::Value`) into the plain
/// session description string that void-sfu expects. Browsers wrap their
/// SDP in `{ "type": "offer", "sdp": "..." }` whereas the legacy native
/// client sometimes sends the raw string. Both shapes are accepted.
fn extract_sdp(value: &serde_json::Value) -> &str {
    if let Some(s) = value.as_str() {
        return s;
    }
    if let Some(obj) = value.as_object() {
        if let Some(sdp) = obj.get("sdp").and_then(|v| v.as_str()) {
            return sdp;
        }
    }
    ""
}

/// Decodes an inbound ICE payload (`RTCIceCandidateInit`-shaped JSON) into
/// the typed [`void_sfu::IceCandidate`]. Returns `None` when the payload
/// does not carry the mandatory `candidate` attribute string.
fn parse_ice_candidate(value: &serde_json::Value) -> Option<void_sfu::IceCandidate> {
    let candidate = value.get("candidate").and_then(|v| v.as_str())?.to_string();
    let sdp_mid = value
        .get("sdpMid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let sdp_mline_index = value
        .get("sdpMLineIndex")
        .and_then(|v| v.as_u64())
        .map(|n| n as u16);
    let username_fragment = value
        .get("usernameFragment")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(void_sfu::IceCandidate {
        candidate,
        sdp_mid,
        sdp_mline_index,
        username_fragment,
    })
}
