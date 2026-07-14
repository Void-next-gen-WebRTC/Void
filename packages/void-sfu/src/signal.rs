// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

use async_trait::async_trait;

use crate::error::SfuResult;
use crate::id::{PeerId, RoomId};
use crate::models::IceCandidate;

/// Outbound message addressed at a single peer.
///
/// The SFU itself is wire-format agnostic: it only describes *what* should be
/// delivered to *which* peer. The consumer implements [`SignalSink`] and is
/// responsible for serialization and transport (WebSocket, QUIC, MQTT, …).
///
/// SDP payloads are exposed as plain `String` (the W3C-defined session
/// description text) and ICE candidates as the typed [`IceCandidate`]
/// the public API never leaks `serde_json` types.
#[derive(Debug, Clone)]
pub enum Outbound {
    /// SDP offer (server-initiated re-negotiation).
    Offer { sdp: String },
    /// SDP answer to a peer-issued offer.
    Answer { sdp: String },
    /// ICE candidate produced by the server-side PeerConnection.
    Ice { candidate: IceCandidate },
    /// Mapping between an RTP track/stream id and the originating peer.
    /// Required by clients that want to associate `ontrack` events with
    /// users without parsing the SDP `a=msid` line themselves. Hosts that
    /// derive the mapping from SDP can ignore this variant.
    TrackMap {
        source_peer: PeerId,
        track_id: String,
        stream_id: String,
        kind: String,
    },
}

/// Implemented by the host application to deliver outbound messages.
///
/// All methods are async to allow back-pressure-aware transports.
/// Implementations call [`SignalSink::deliver`] from internal SFU paths
/// whenever an [`Outbound`] needs to leave the server.
#[async_trait]
pub trait SignalSink: Send + Sync + 'static {
    /// Sends an outbound payload to a single peer.
    ///
    /// Implementations should enqueue the message on the peer's transport
    /// channel and return promptly. A non-fatal delivery failure (e.g. queue
    /// full) should be reported as `Ok(())` after recording it on the host
    /// side; only return `Err` for unrecoverable conditions that warrant
    /// peer eviction.
    async fn deliver(&self, peer: &PeerId, message: Outbound) -> SfuResult<()>;
}

/// Notifications about room membership changes the SFU emits to the host.
///
/// Consumed via [`crate::Sfu`] callbacks if the host installs an observer.
/// Kept separate from [`Outbound`] to make clear these are *fan-out* events,
/// Notifications about room membership changes the SFU emits to the host.
#[derive(Debug, Clone)]
pub enum RoomEvent {
    PeerJoined {
        room: RoomId,
        peer: PeerId,
    },
    PeerLeft {
        room: RoomId,
        peer: PeerId,
    },
    /// A peer opened a data channel that is now being relayed inside the
    /// room. The host typically uses this to mirror state into its own
    /// presence model or to wire feature flags around the channel label.
    DataChannelOpened {
        room: RoomId,
        peer: PeerId,
        label: String,
    },
    /// A previously relayed data channel has been torn down (publisher
    /// closed it, or its peer left the room).
    DataChannelClosed {
        room: RoomId,
        peer: PeerId,
        label: String,
    },
}

/// Optional observer for room events. Useful for the host to keep its own
/// presence/broadcast state in sync without re-deriving it from SFU calls.
#[async_trait]
pub trait RoomObserver: Send + Sync + 'static {
    async fn on_event(&self, event: RoomEvent);
}
