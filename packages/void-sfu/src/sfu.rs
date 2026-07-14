// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

use std::sync::Arc;
use std::time::SystemTime;

use dashmap::DashMap;
use tracing::{debug, warn};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::api::API;

use crate::config::SfuConfig;
use crate::error::{SfuError, SfuResult};
use crate::forwarder::PeerEntry;
use crate::id::{PeerId, RoomId};
use crate::room::{RoomPeer, RoomState};
use crate::signal::{RoomEvent, RoomObserver, SignalSink};

/// Internal shared state. Held inside [`Sfu`] as `Arc<SfuInner>` so it can be
/// freely cloned into background tasks (RTP forwarding workers, on_track
/// callbacks, renegotiation tasks) without copying.
pub(crate) struct SfuInner {
    pub config: SfuConfig,
    pub api: API,
    pub peers: DashMap<PeerId, Arc<PeerEntry>>,
    pub rooms: DashMap<RoomId, Arc<RoomState>>,
    pub observer: parking_lot::RwLock<Option<Arc<dyn RoomObserver>>>,
}

/// Selective Forwarding Unit handle.
///
/// `Sfu` is `Clone`-cheap (just an `Arc` bump). All public methods are
/// thread-safe and return `Result<_, SfuError>` no panics, no unwraps.
#[derive(Clone)]
pub struct Sfu {
    pub(crate) inner: Arc<SfuInner>,
}

impl Sfu {
    /// Creates a new SFU using the supplied configuration and a default
    /// webrtc-rs `API` (default codecs + default interceptors).
    ///
    /// For custom UDP port ranges or media engines, use [`Sfu::with_api`].
    pub fn new(config: SfuConfig) -> SfuResult<Self> {
        let mut media = MediaEngine::default();
        media.register_default_codecs().map_err(SfuError::WebRtc)?;

        let mut registry = webrtc::interceptor::registry::Registry::default();
        registry = register_default_interceptors(registry, &mut media).map_err(SfuError::WebRtc)?;

        let api = APIBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .build();

        Ok(Self::with_api(config, api))
    }

    /// Creates a new SFU using a pre-built webrtc-rs `API`.
    ///
    /// Use this when you need a custom `SettingEngine` (e.g. ephemeral UDP
    /// port range) or registered codecs beyond the defaults.
    pub fn with_api(config: SfuConfig, api: API) -> Self {
        Self {
            inner: Arc::new(SfuInner {
                config,
                api,
                peers: DashMap::new(),
                rooms: DashMap::new(),
                observer: parking_lot::RwLock::new(None),
            }),
        }
    }

    /// Installs a [`RoomObserver`] receiving membership change events.
    ///
    /// Replaces any previously installed observer.
    pub fn set_observer(&self, observer: Arc<dyn RoomObserver>) {
        *self.inner.observer.write() = Some(observer);
    }

    /// Registers a peer with its [`SignalSink`].
    ///
    /// Returns [`SfuError::PeerAlreadyExists`] if a peer with the same id
    /// is already registered.
    pub fn add_peer(&self, peer_id: PeerId, sink: Arc<dyn SignalSink>) -> SfuResult<()> {
        if self.inner.peers.contains_key(&peer_id) {
            return Err(SfuError::PeerAlreadyExists(peer_id.as_arc()));
        }
        let entry = Arc::new(PeerEntry::new(peer_id.clone(), sink));
        self.inner.peers.insert(peer_id, entry);
        Ok(())
    }

    /// Removes a peer, closing its PeerConnection and removing it from any
    /// room. Notifies remaining members via the room observer.
    pub async fn remove_peer(&self, peer_id: &PeerId) -> SfuResult<()> {
        let Some((_, entry)) = self.inner.peers.remove(peer_id) else {
            return Err(SfuError::PeerNotFound(peer_id.as_arc()));
        };

        // Close PC if any.
        let pc_opt = entry.peer_connection.lock().await.take();
        if let Some(pc) = pc_opt {
            if let Err(e) = pc.close().await {
                debug!("pc.close failed for {}: {:?}", peer_id, e);
            }
        }

        let room_id_opt = entry.room.read().clone();
        if let Some(room_id) = room_id_opt {
            self.detach_from_room(peer_id, &room_id).await;
            self.notify(RoomEvent::PeerLeft {
                room: room_id,
                peer: peer_id.clone(),
            })
            .await;
        }
        Ok(())
    }

    /// Joins (or moves) a peer into a room.
    ///
    /// Returns the snapshot of pre-existing peers (excluding the joiner) so
    /// the host can build its own `joined`/membership message.
    pub async fn join_room(&self, peer_id: &PeerId, room_id: RoomId) -> SfuResult<JoinSnapshot> {
        // Resolve peer entry.
        let entry = self
            .inner
            .peers
            .get(peer_id)
            .ok_or_else(|| SfuError::PeerNotFound(peer_id.as_arc()))?
            .value()
            .clone();

        // Detach from previous room (if any).
        let previous = entry.room.read().clone();
        if let Some(prev) = previous {
            if prev != room_id {
                self.detach_from_room(peer_id, &prev).await;
                self.notify(RoomEvent::PeerLeft {
                    room: prev,
                    peer: peer_id.clone(),
                })
                .await;
            }
        }

        // Get-or-create the target room.
        let room = self
            .inner
            .rooms
            .entry(room_id.clone())
            .or_insert_with(|| Arc::new(RoomState::new(room_id.clone(), now_ms())))
            .value()
            .clone();

        // Snapshot existing members BEFORE inserting the joiner.
        let existing: Vec<RoomPeer> = room
            .members
            .iter()
            .filter(|m| m.as_str() != peer_id.as_str())
            .map(|m| RoomPeer { peer_id: m.clone() })
            .collect();

        room.members.insert(peer_id.clone());
        *entry.room.write() = Some(room_id.clone());

        let snapshot = JoinSnapshot {
            room_id: room_id.clone(),
            started_at_ms: room.started_at_ms,
            existing_peers: existing,
        };

        self.notify(RoomEvent::PeerJoined {
            room: room_id,
            peer: peer_id.clone(),
        })
        .await;

        Ok(snapshot)
    }

    /// Removes a peer from its current room (if any). Idempotent.
    pub async fn leave_room(&self, peer_id: &PeerId) -> SfuResult<()> {
        let entry = self
            .inner
            .peers
            .get(peer_id)
            .ok_or_else(|| SfuError::PeerNotFound(peer_id.as_arc()))?
            .value()
            .clone();
        let room_opt = entry.room.write().take();
        if let Some(room_id) = room_opt {
            self.detach_from_room(peer_id, &room_id).await;
            self.notify(RoomEvent::PeerLeft {
                room: room_id,
                peer: peer_id.clone(),
            })
            .await;
        }
        Ok(())
    }

    /// Forwards an SDP offer for the named peer to the SFU's negotiation
    /// pipeline. The answer is delivered asynchronously through the peer's
    /// [`SignalSink`].
    pub async fn handle_offer(&self, peer_id: &PeerId, sdp: &str) -> SfuResult<()> {
        crate::negotiation::handle_offer(Arc::clone(&self.inner), peer_id.clone(), sdp).await
    }

    /// Applies an SDP answer (typically a response to a server-initiated
    /// renegotiation offer).
    pub async fn handle_answer(&self, peer_id: &PeerId, sdp: &str) -> SfuResult<()> {
        crate::negotiation::handle_answer(Arc::clone(&self.inner), peer_id.clone(), sdp).await
    }

    /// Adds a remote ICE candidate to the peer's PC.
    pub async fn handle_ice(
        &self,
        peer_id: &PeerId,
        candidate: crate::models::IceCandidate,
    ) -> SfuResult<()> {
        crate::negotiation::handle_ice(Arc::clone(&self.inner), peer_id.clone(), candidate).await
    }

    /// Snapshots the member ids of a room. Returns an empty vec if the room
    /// does not exist. The result is a `Vec` (not an iterator borrowing the
    /// concurrent map) so the caller can `await` after dropping the snapshot.
    pub fn room_members(&self, room_id: &RoomId) -> Vec<PeerId> {
        let Some(room) = self.inner.rooms.get(room_id) else {
            return Vec::new();
        };
        room.value().members.iter().map(|m| m.clone()).collect()
    }

    /// Returns the room a peer is currently in (if any). Cheap clone.
    pub fn peer_room(&self, peer_id: &PeerId) -> Option<RoomId> {
        self.inner
            .peers
            .get(peer_id)
            .and_then(|e| e.value().room.read().clone())
    }

    /// Number of registered peers.
    #[inline]
    pub fn peer_count(&self) -> usize {
        self.inner.peers.len()
    }

    /// Number of active rooms.
    #[inline]
    pub fn room_count(&self) -> usize {
        self.inner.rooms.len()
    }

    // ---- Internal ----

    async fn detach_from_room(&self, peer_id: &PeerId, room_id: &RoomId) {
        let Some(room_ref) = self.inner.rooms.get(room_id) else {
            return;
        };
        let room = Arc::clone(room_ref.value());
        drop(room_ref);

        room.members.remove(peer_id);

        // Drop forwarders the peer was the source of, and remove this peer
        // as a destination from all remaining forwarders.
        room.forwarders.retain(|_, f| f.source_peer != *peer_id);
        for kv in room.forwarders.iter() {
            kv.value().destination_tracks.remove(peer_id);
        }

        // Mirror the same cleanup for data-channel forwarders.
        room.dc_forwarders.retain(|_, f| f.source_peer != *peer_id);
        for kv in room.dc_forwarders.iter() {
            kv.value().destination_channels.remove(peer_id);
        }

        // Garbage-collect empty rooms.
        if room.members.is_empty() {
            self.inner.rooms.remove(room_id);
        }
    }

    async fn notify(&self, event: RoomEvent) {
        let observer = self.inner.observer.read().clone();
        if let Some(obs) = observer {
            obs.on_event(event).await;
        }
    }
}

/// Snapshot returned by [`Sfu::join_room`].
#[derive(Debug, Clone)]
pub struct JoinSnapshot {
    pub room_id: RoomId,
    pub started_at_ms: u64,
    pub existing_peers: Vec<RoomPeer>,
}

#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_else(|_| {
            warn!("system clock before UNIX_EPOCH; using 0");
            0
        })
}
