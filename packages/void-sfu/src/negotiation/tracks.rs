// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Track installation: registers per-source forwarders, broadcasts the
//! `TrackMap` mapping (legacy convenience event) and attaches destination
//! tracks to existing room members.
//!
//! The hot RTP fan-out path itself lives in [`super::fanout`]; this module
//! is in charge of bookkeeping that runs once per published track.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use webrtc::rtp::packet::Packet;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;

use crate::forwarder::ForwarderState;
use crate::id::{MediaSourceId, PeerId, RoomId};
use crate::sfu::SfuInner;
use crate::signal::Outbound;

use super::fanout::{spawn_fan_out_worker, spawn_source_reader};
use super::renegotiate::spawn_renegotiation_offer;

/// Wires `on_track` to register a forwarder and fan out RTP packets to all
/// other peers currently in the same room.
pub(super) fn install_on_track(
    pc: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    inner: Arc<SfuInner>,
    source_peer: PeerId,
    room_id: RoomId,
) {
    pc.on_track(Box::new(move |track, _, _| {
        let inner = Arc::clone(&inner);
        let source_peer = source_peer.clone();
        let room_id = room_id.clone();
        Box::pin(async move {
            let track_id: Arc<str> = Arc::from(track.id());
            let stream_id: Arc<str> = Arc::from(track.stream_id());
            let kind: Arc<str> = Arc::from(track.kind().to_string());
            let codec = track.codec().capability.clone();
            let ssrc = track.ssrc();
            let media_source_id =
                MediaSourceId::from_peer_and_track(&source_peer, track_id.as_ref());

            info!(
                "on_track source={} room={} track_id={} stream_id={} kind={} ssrc={}",
                source_peer,
                room_id,
                track_id,
                stream_id,
                kind,
                ssrc
            );

            let (tx_track, rx_track) = mpsc::channel::<Packet>(inner.config.rtp_channel_capacity);

            let dest_tracks = Arc::new(DashMap::<PeerId, Arc<TrackLocalStaticRTP>>::new());

            let forwarder = Arc::new(ForwarderState {
                media_source_id: media_source_id.clone(),
                source_peer: source_peer.clone(),
                track_id: Arc::clone(&track_id),
                stream_id: Arc::clone(&stream_id),
                kind: Arc::clone(&kind),
                codec: codec.clone(),
                ssrc,
                destination_tracks: Arc::clone(&dest_tracks),
                tx: tx_track.clone(),
            });

            // Register the forwarder atomically.
            let room = match inner.rooms.get(&room_id) {
                Some(r) => Arc::clone(r.value()),
                None => {
                    warn!("on_track fired for missing room {}", room_id);
                    return;
                }
            };
            room.forwarders
                .insert(media_source_id.clone(), Arc::clone(&forwarder));

            // Notify every member (including the publisher itself, which
            // uses the mapping to label its outbound stream client-side).
            broadcast_track_map(&inner, &room, &source_peer, &track_id, &stream_id, &kind).await;

            // Add a destination track to each existing member (excl. publisher).
            attach_destinations_to_existing_members(
                &inner,
                &room,
                &source_peer,
                &codec,
                &track_id,
                &stream_id,
                &dest_tracks,
            )
            .await;

            // Source RTP reader -> jitter buffer -> bounded channel.
            spawn_source_reader(track, tx_track, &inner.config, codec.clone());

            // Bounded channel -> fan-out to destination tracks.
            spawn_fan_out_worker(
                Arc::clone(&inner),
                room_id.clone(),
                source_peer.clone(),
                media_source_id.clone(),
                Arc::clone(&kind),
                Arc::clone(&dest_tracks),
                rx_track,
            );
        })
    }));
}

async fn broadcast_track_map(
    inner: &Arc<SfuInner>,
    room: &Arc<crate::room::RoomState>,
    source_peer: &PeerId,
    track_id: &Arc<str>,
    stream_id: &Arc<str>,
    kind: &Arc<str>,
) {
    // Snapshot member ids; release the iterator before awaiting on sinks.
    let members: Vec<PeerId> = room.members.iter().map(|m| m.clone()).collect();
    for member in members {
        let Some(entry) = inner.peers.get(&member) else {
            continue;
        };
        let entry = Arc::clone(entry.value());
        let outbound = Outbound::TrackMap {
            source_peer: source_peer.clone(),
            track_id: track_id.to_string(),
            stream_id: stream_id.to_string(),
            kind: kind.to_string(),
        };
        if let Err(e) = entry.sink.deliver(&member, outbound).await {
            debug!("track-map delivery failed for {}: {:?}", member, e);
        }
    }
}

async fn attach_destinations_to_existing_members(
    inner: &Arc<SfuInner>,
    room: &Arc<crate::room::RoomState>,
    source_peer: &PeerId,
    codec: &RTCRtpCodecCapability,
    track_id: &Arc<str>,
    stream_id: &Arc<str>,
    dest_tracks: &Arc<DashMap<PeerId, Arc<TrackLocalStaticRTP>>>,
) {
    let members: Vec<PeerId> = room
        .members
        .iter()
        .map(|m| m.clone())
        .filter(|m| m != source_peer)
        .collect();

    for member in members {
        let dest_track = Arc::new(TrackLocalStaticRTP::new(
            codec.clone(),
            track_id.to_string(),
            stream_id.to_string(),
        ));
        dest_tracks.insert(member.clone(), Arc::clone(&dest_track));

        let Some(entry) = inner.peers.get(&member) else {
            continue;
        };
        let entry = Arc::clone(entry.value());
        let pc_opt = entry.peer_connection.lock().await.clone();
        let Some(other_pc) = pc_opt else { continue };

        if let Err(e) = other_pc
            .add_transceiver_from_track(
                Arc::clone(&dest_track) as Arc<dyn TrackLocal + Send + Sync>,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Sendonly,
                    send_encodings: Vec::new(),
                }),
            )
            .await
        {
            // See the matching comment in catchup.rs: `add_track` would
            // silently reuse a recvonly transceiver in webrtc-rs and emit a
            // renegotiation offer without the new m-line, isolating peers.
            warn!("add_transceiver_from_track to {} failed: {:?}", member, e);
            continue;
        }

        info!(
            "destination attached source={} -> member={} track_id={} stream_id={}",
            source_peer,
            member,
            track_id,
            stream_id
        );

        // Renegotiate with the existing member.
        let other_pc = Arc::clone(&other_pc);
        let sink = Arc::clone(&entry.sink);
        let member_id = member.clone();
        tokio::spawn(async move {
            spawn_renegotiation_offer(other_pc, sink, member_id).await;
        });
    }
}
