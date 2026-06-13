// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! "Catchup": when a peer joins a room with already-publishing members,
//! attach existing forwarders' tracks to the joiner's PC and renegotiate
//! once with all of them at the same time.

use std::sync::Arc;

use tracing::{debug, info, warn};
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;

use crate::error::{SfuError, SfuResult};
use crate::forwarder::ForwarderState;
use crate::id::{PeerId, RoomId};
use crate::sfu::SfuInner;
use crate::signal::Outbound;

use super::renegotiate::spawn_renegotiation_offer;

/// Adds existing source tracks of the room to a freshly-joined peer's PC,
/// then issues a *single* renegotiation offer to it.
pub(super) async fn catchup_existing_tracks(
    inner: Arc<SfuInner>,
    peer_id: PeerId,
    room_id: RoomId,
    pc: Arc<webrtc::peer_connection::RTCPeerConnection>,
) -> SfuResult<()> {
    let Some(room_ref) = inner.rooms.get(&room_id) else {
        return Ok(()); // Room may have been torn down already.
    };
    let room = Arc::clone(room_ref.value());
    drop(room_ref);

    // Snapshot forwarders to avoid holding the DashMap shard guard across awaits.
    struct CatchupItem {
        source_peer: PeerId,
        codec: RTCRtpCodecCapability,
        track_id: Arc<str>,
        stream_id: Arc<str>,
        kind: Arc<str>,
        forwarder: Arc<ForwarderState>,
    }

    // Catch up every published media source in the room except those
    // belonging to the joining peer itself (a peer never receives its
    // own outbound tracks).
    let items: Vec<CatchupItem> = room
        .forwarders
        .iter()
        .filter(|kv| kv.value().source_peer != peer_id)
        .map(|kv| {
            let f = kv.value();
            CatchupItem {
                source_peer: f.source_peer.clone(),
                codec: f.codec.clone(),
                track_id: Arc::clone(&f.track_id),
                stream_id: Arc::clone(&f.stream_id),
                kind: Arc::clone(&f.kind),
                forwarder: Arc::clone(f),
            }
        })
        .collect();

    let has_dc_to_catch = room
        .dc_forwarders
        .iter()
        .any(|kv| kv.value().source_peer != peer_id);
    if items.is_empty() && !has_dc_to_catch {
        return Ok(());
    }

    let peer_entry = inner
        .peers
        .get(&peer_id)
        .ok_or_else(|| SfuError::PeerNotFound(peer_id.as_arc()))?
        .value()
        .clone();

    for item in items {
        // Tell the new peer how to label the soon-to-arrive stream.
        let outbound = Outbound::TrackMap {
            source_peer: item.source_peer.clone(),
            track_id: item.track_id.to_string(),
            stream_id: item.stream_id.to_string(),
            kind: item.kind.to_string(),
        };
        if let Err(e) = peer_entry.sink.deliver(&peer_id, outbound).await {
            debug!("catchup track-map delivery failed: {:?}", e);
        }

        let dest_track = Arc::new(TrackLocalStaticRTP::new(
            item.codec,
            item.track_id.to_string(),
            item.stream_id.to_string(),
        ));
        item.forwarder
            .destination_tracks
            .insert(peer_id.clone(), Arc::clone(&dest_track));

        if let Err(e) = pc
            .add_transceiver_from_track(
                Arc::clone(&dest_track) as Arc<dyn TrackLocal + Send + Sync>,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Sendonly,
                    send_encodings: Vec::new(),
                }),
            )
            .await
        {
            // Falling back to add_track here would silently reuse a recvonly
            // transceiver in some webrtc-rs paths, producing a renegotiation
            // offer with no new m-line — exactly the "peers see each other
            // but hear nothing" symptom. We surface the error instead.
            warn!("catchup add_transceiver_from_track failed: {:?}", e);
        } else {
            info!(
                "catchup destination attached source={} -> peer={} track_id={} stream_id={}",
                item.source_peer,
                peer_id,
                item.track_id,
                item.stream_id
            );
        }
    }

    // ---- Catch up published data channels ----
    // For each forwarder in the room not authored by the joiner, create a
    // matching local data channel on its PC and register it as destination.
    // The renegotiation offer below covers SCTP m-line additions too.
    for kv in room.dc_forwarders.iter() {
        let forwarder = Arc::clone(kv.value());
        if forwarder.source_peer == peer_id {
            continue;
        }
        let init = webrtc::data_channel::data_channel_init::RTCDataChannelInit {
            ordered: Some(forwarder.ordered),
            max_packet_life_time: forwarder.max_packet_life_time,
            max_retransmits: forwarder.max_retransmits,
            protocol: Some(forwarder.protocol.to_string()),
            ..Default::default()
        };
        match pc
            .create_data_channel(forwarder.label.as_ref(), Some(init))
            .await
        {
            Ok(local_dc) => {
                forwarder
                    .destination_channels
                    .insert(peer_id.clone(), local_dc);
            }
            Err(e) => {
                warn!(
                    "catchup create_data_channel failed for label={}: {:?}",
                    forwarder.label, e
                );
            }
        }
    }

    // Single renegotiation offer covering all just-added tracks and data
    // channels. Spawned to avoid blocking the caller while keeping ordering
    // through the per-peer outbound mpsc: the answer was already enqueued
    // upstream, so a polite client always receives it before this offer.
    let sink = Arc::clone(&peer_entry.sink);
    let peer_id_clone = peer_id.clone();
    info!("catchup renegotiation scheduled for peer={}", peer_id);
    tokio::spawn(async move {
        spawn_renegotiation_offer(pc, sink, peer_id_clone).await;
    });

    Ok(())
}
