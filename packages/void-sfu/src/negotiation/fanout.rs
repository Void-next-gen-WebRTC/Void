// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! RTP fan-out workers and per-source jitter buffer reader.
//!
//! This module owns the hot data path: every incoming RTP packet flows
//! through [`spawn_source_reader`] (jitter-buffered) into a bounded mpsc
//! channel, then through [`spawn_fan_out_worker`] which evaluates the
//! configured [`PacketInterceptor`] chain on ingress and on egress before
//! writing to each destination track.

use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::debug;
use webrtc::rtp::packet::Packet;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocalWriter;

use crate::extension::{Direction, InterceptOutcome, PacketContext, PacketInterceptor};
use crate::id::{MediaSourceId, PeerId, RoomId};
use crate::jitter::JitterBuffer;
use crate::sfu::SfuInner;
use crate::stats::ForwardingStats;

/// Reads RTP packets from a remote track, runs them through a jitter buffer
/// keyed by the negotiated codec clock rate, and forwards them on a bounded
/// mpsc channel consumed by [`spawn_fan_out_worker`].
pub(super) fn spawn_source_reader(
    track: Arc<webrtc::track::track_remote::TrackRemote>,
    tx_track: mpsc::Sender<Packet>,
    config: &crate::config::SfuConfig,
    codec: RTCRtpCodecCapability,
) {
    // Per-track jitter parameters: derive clock rate from the negotiated
    // codec capability (Opus 48k, VP8/VP9/H264/AV1 90k, …) and allow a
    // host-supplied override keyed by MIME type.
    let mime_lower = codec.mime_type.to_ascii_lowercase();
    let override_policy = config
        .jitter_overrides
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&mime_lower))
        .map(|(_, v)| v.clone());

    let playout = override_policy
        .as_ref()
        .map(|p| p.playout_ms)
        .unwrap_or(config.jitter_playout_ms);
    let clock = override_policy
        .as_ref()
        .and_then(|p| p.clock_rate)
        .unwrap_or(codec.clock_rate);

    tokio::spawn(async move {
        let mut jitter = JitterBuffer::new(playout, clock);
        while let Ok((packet, _)) = track.read_rtp().await {
            jitter.push(packet);
            while let Some(p) = jitter.pop() {
                if tx_track.try_send(p).is_err() {
                    // Bounded channel full — packet dropped on purpose to
                    // protect the worker. Hosts can wire a metric later.
                    debug!("RTP fan-in queue full; dropping packet");
                }
            }
        }
    });
}

/// Drains the bounded RTP channel and writes each packet to every active
/// destination track for the source. Runs the configured interceptor chain
/// twice — once at ingress, once per destination — short-circuiting when
/// no interceptor is registered to keep the empty-config fast path
/// branchless beyond the `is_empty` check.
pub(super) fn spawn_fan_out_worker(
    inner: Arc<SfuInner>,
    room_id: RoomId,
    source_peer: PeerId,
    media_source_id: MediaSourceId,
    kind: Arc<str>,
    dest_tracks: Arc<DashMap<PeerId, Arc<TrackLocalStaticRTP>>>,
    mut rx: mpsc::Receiver<Packet>,
) {
    let flush_interval = inner.config.stats_flush_interval;
    tokio::spawn(async move {
        let local_stats: DashMap<PeerId, (u64, u64)> = DashMap::new();
        let mut last_flush = Instant::now();

        while let Some(mut packet) = rx.recv().await {
            // ---- Ingress interceptor pass ----
            if !inner.config.interceptors.is_empty()
                && run_ingress(
                    &inner.config.interceptors,
                    &media_source_id,
                    &kind,
                    &mut packet,
                )
                .await
            {
                continue;
            }

            let payload_len = packet.payload.len() as u64;
            if dest_tracks.is_empty() {
                continue;
            }

            for entry in dest_tracks.iter() {
                let dest_user = entry.key().clone();
                let track = Arc::clone(entry.value());

                // ---- Egress interceptor pass (per destination) ----
                let mut egress_packet = packet.clone();
                if !inner.config.interceptors.is_empty()
                    && run_egress(
                        &inner.config.interceptors,
                        &media_source_id,
                        &kind,
                        &dest_user,
                        &mut egress_packet,
                    )
                    .await
                {
                    continue;
                }

                if let Err(e) = track.write_rtp(&egress_packet).await {
                    debug!("write_rtp to {} failed: {:?}", dest_user, e);
                    continue;
                }
                let mut s = local_stats.entry(dest_user).or_insert((0, 0));
                s.0 += 1;
                s.1 += payload_len;
            }

            if last_flush.elapsed() >= flush_interval && !local_stats.is_empty() {
                flush_local_stats(&inner, &source_peer, &local_stats);
                local_stats.clear();
                last_flush = Instant::now();
            }
        }
        // Drain any pending stats on shutdown.
        if !local_stats.is_empty() {
            flush_local_stats(&inner, &source_peer, &local_stats);
        }
        debug!(
            "fan-out worker exited for source={} room={}",
            source_peer, room_id
        );
    });
}

/// Runs the ingress interceptor chain in order. Returns `true` when the
/// caller must drop the packet entirely (no destination should receive it).
async fn run_ingress(
    interceptors: &[Arc<dyn PacketInterceptor>],
    media_source_id: &MediaSourceId,
    kind: &Arc<str>,
    packet: &mut Packet,
) -> bool {
    for interceptor in interceptors.iter() {
        let ctx = PacketContext {
            source: media_source_id,
            destination: None,
            kind: kind.as_ref(),
            direction: Direction::Ingress,
        };
        match interceptor.on_rtp(ctx, packet).await {
            InterceptOutcome::Forward => {}
            InterceptOutcome::Drop => return true,
            InterceptOutcome::Replace(new_pkt) => *packet = new_pkt,
        }
    }
    false
}

/// Runs the egress interceptor chain for a single destination. Returns
/// `true` when this specific destination must be skipped (others continue).
async fn run_egress(
    interceptors: &[Arc<dyn PacketInterceptor>],
    media_source_id: &MediaSourceId,
    kind: &Arc<str>,
    dest_user: &PeerId,
    packet: &mut Packet,
) -> bool {
    for interceptor in interceptors.iter() {
        let ctx = PacketContext {
            source: media_source_id,
            destination: Some(dest_user),
            kind: kind.as_ref(),
            direction: Direction::Egress,
        };
        match interceptor.on_rtp(ctx, packet).await {
            InterceptOutcome::Forward => {}
            InterceptOutcome::Drop => return true,
            InterceptOutcome::Replace(new_pkt) => *packet = new_pkt,
        }
    }
    false
}

fn flush_local_stats(
    inner: &Arc<SfuInner>,
    source_peer: &PeerId,
    local_stats: &DashMap<PeerId, (u64, u64)>,
) {
    let Some(source_entry) = inner.peers.get(source_peer) else {
        return;
    };
    for kv in local_stats.iter() {
        let dest = kv.key().clone();
        let (pkts, bytes) = *kv.value();
        let mut s = source_entry
            .stats
            .entry(dest)
            .or_insert_with(ForwardingStats::new);
        s.update(pkts, bytes);
    }
}
