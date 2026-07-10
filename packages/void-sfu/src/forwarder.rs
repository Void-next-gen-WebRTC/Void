// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{mpsc, Mutex};
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp::packet::Packet;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;

use crate::id::{MediaSourceId, PeerId};
use crate::stats::ForwardingStats;

/// Per-source-track forwarder.
///
/// Owns the destination tracks (one per consuming peer) and the bounded
/// channel that the source's RTP reader feeds. The forwarding worker
/// (spawned in [`crate::negotiation::on_track`]) drains the channel and
/// fans out packets to every destination track without ever copying the
/// payload bytes (webrtc-rs's `Packet` carries a `bytes::Bytes` payload
/// which is `Arc`-backed).
#[allow(dead_code)] // Several fields are kept for diagnostics / future routing.
pub(crate) struct ForwarderState {
    /// Stable identifier for this published track (peer + track_id).
    pub media_source_id: MediaSourceId,
    /// User id of the publisher.
    pub source_peer: PeerId,
    pub track_id: Arc<str>,
    pub stream_id: Arc<str>,
    pub kind: Arc<str>,
    pub codec: RTCRtpCodecCapability,
    /// Synchronization Source identifier of the publisher's RTP track.
    /// Required when synthesising RTCP feedback (PLI/FIR/NACK) addressed
    /// at this source.
    pub ssrc: u32,
    /// Fine-grained map keyed by destination peer id.
    pub destination_tracks: Arc<DashMap<PeerId, Arc<TrackLocalStaticRTP>>>,
    /// Bounded RTP packet channel; dropped packets are accounted for via
    /// the `on_packet_dropped` host callback (if installed).
    pub tx: mpsc::Sender<Packet>,
}

/// Internal handle to a peer's media session.
pub(crate) struct PeerEntry {
    pub id: PeerId,
    pub room: parking_lot::RwLock<Option<crate::id::RoomId>>,
    pub peer_connection: Mutex<Option<Arc<RTCPeerConnection>>>,
    pub sink: Arc<dyn crate::signal::SignalSink>,
    /// Per-destination forwarding stats (updated in batches).
    pub stats: DashMap<PeerId, ForwardingStats>,
}

impl PeerEntry {
    pub fn new(id: PeerId, sink: Arc<dyn crate::signal::SignalSink>) -> Self {
        Self {
            id,
            room: parking_lot::RwLock::new(None),
            peer_connection: Mutex::new(None),
            sink,
            stats: DashMap::new(),
        }
    }
}
