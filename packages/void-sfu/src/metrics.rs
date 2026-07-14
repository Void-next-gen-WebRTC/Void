// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Aggregate metrics exposed by the SFU.
//!
//! Hosts call [`Sfu::metrics_snapshot`] on a slow tick (>= 1 s) and convert
//! the structured snapshot to whatever observability format they use. The
//! library never owns the exporter — staying transport- and stack-agnostic.

use crate::id::PeerId;
use crate::sfu::Sfu;

/// Aggregate metrics snapshot returned by [`Sfu::metrics_snapshot`].
#[derive(Debug, Clone, Default)]
pub struct MetricsSnapshot {
    pub peer_count: usize,
    pub room_count: usize,
    pub peer_connections: usize,
    pub total_forwarders: usize,
    pub total_dest_tracks: usize,
    pub members_per_room: Vec<usize>,
    pub peer_bandwidths: Vec<(PeerId, u64)>,
}

impl Sfu {
    /// Returns the bandwidth (bps) the SFU has been forwarding *from* a peer
    /// across all destinations, aggregated since the last reset. Useful for
    /// the host to expose its own metrics.
    pub fn aggregated_bandwidth_bps(&self, peer_id: &PeerId) -> u64 {
        let Some(entry) = self.inner.peers.get(peer_id) else {
            return 0;
        };
        entry
            .stats
            .iter()
            .map(|kv| kv.value().bandwidth_bps())
            .sum()
    }

    /// Aggregate snapshot for metrics exposition. Allocates once; the host
    /// is expected to call this on a slow tick (>= 1 s).
    pub async fn metrics_snapshot(&self) -> MetricsSnapshot {
        let mut total_forwarders = 0usize;
        let mut total_dest_tracks = 0usize;
        let mut members_per_room: Vec<usize> = Vec::with_capacity(self.inner.rooms.len());
        for kv in self.inner.rooms.iter() {
            let room = kv.value();
            members_per_room.push(room.members.len());
            total_forwarders += room.forwarders.len();
            for f in room.forwarders.iter() {
                total_dest_tracks += f.value().destination_tracks.len();
            }
        }

        let mut peer_connections = 0usize;
        let mut peer_bandwidths: Vec<(PeerId, u64)> = Vec::with_capacity(self.inner.peers.len());
        for kv in self.inner.peers.iter() {
            let entry = kv.value();
            if entry.peer_connection.lock().await.is_some() {
                peer_connections += 1;
            }
            let bps: u64 = entry.stats.iter().map(|s| s.value().bandwidth_bps()).sum();
            peer_bandwidths.push((entry.id.clone(), bps));
        }

        MetricsSnapshot {
            peer_count: self.inner.peers.len(),
            room_count: self.inner.rooms.len(),
            peer_connections,
            total_forwarders,
            total_dest_tracks,
            members_per_room,
            peer_bandwidths,
        }
    }
}
