// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! SCTP data channel installation and forwarding.
//!
//! Symmetric to [`super::tracks`] but for `RTCDataChannel`s. When a peer
//! opens a data channel, the SFU:
//! 1. Registers a [`DataChannelForwarder`] in the room.
//! 2. For each existing room member, creates a matching local data channel
//!    on their PC (which triggers an SDP renegotiation).
//! 3. Spawns a forwarder task that runs every received message through the
//!    configured [`DataChannelInterceptor`] chain (ingress + per-egress)
//!    and writes it on every destination channel.

use std::sync::Arc;

use bytes::Bytes;
use dashmap::DashMap;
use tracing::{debug, warn};
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::RTCDataChannel;

use crate::dc_forwarder::DataChannelForwarder;
use crate::id::{DataChannelSourceId, PeerId, RoomId};
use crate::sfu::SfuInner;
use crate::signal::RoomEvent;

use super::dc_interceptors::{run_dc_egress, run_dc_ingress};
use super::renegotiate::spawn_renegotiation_offer;

/// Wires `on_data_channel` so any DC the publisher opens is fanned out to
/// every other room member.
pub(super) fn install_on_data_channel(
    pc: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    inner: Arc<SfuInner>,
    source_peer: PeerId,
    room_id: RoomId,
) {
    pc.on_data_channel(Box::new(move |dc| {
        let inner = Arc::clone(&inner);
        let source_peer = source_peer.clone();
        let room_id = room_id.clone();
        Box::pin(async move {
            register_data_channel(inner, source_peer, room_id, dc).await;
        })
    }));
}

async fn register_data_channel(
    inner: Arc<SfuInner>,
    source_peer: PeerId,
    room_id: RoomId,
    dc: Arc<RTCDataChannel>,
) {
    let label: Arc<str> = Arc::from(dc.label());
    let protocol: Arc<str> = Arc::from(dc.protocol());
    let ordered = dc.ordered();
    let max_packet_life_time = dc.max_packet_lifetime();
    let max_retransmits = dc.max_retransmits();
    let source_id = DataChannelSourceId::from_peer_and_label(&source_peer, &label);

    let dest_channels = Arc::new(DashMap::<PeerId, Arc<RTCDataChannel>>::new());

    let forwarder = Arc::new(DataChannelForwarder {
        source_id: source_id.clone(),
        source_peer: source_peer.clone(),
        label: Arc::clone(&label),
        ordered,
        max_packet_life_time,
        max_retransmits,
        protocol,
        destination_channels: Arc::clone(&dest_channels),
    });

    // Register before fan-out so concurrent catchups see the forwarder.
    let Some(room_ref) = inner.rooms.get(&room_id) else {
        warn!("on_data_channel fired for missing room {}", room_id);
        return;
    };
    let room = Arc::clone(room_ref.value());
    drop(room_ref);
    room.dc_forwarders
        .insert(source_id.clone(), Arc::clone(&forwarder));

    // Notify the host (presence/feature mirroring).
    notify_dc_opened(&inner, &room_id, &source_peer, &label).await;

    // Attach a destination DC on each existing peer (excluding publisher).
    attach_dc_destinations_to_existing_members(&inner, &room, &source_peer, &forwarder).await;

    // Forward every message arriving on the publisher's DC.
    install_message_forwarder(Arc::clone(&inner), Arc::clone(&forwarder), dc, source_id);
}

async fn notify_dc_opened(
    inner: &Arc<SfuInner>,
    room_id: &RoomId,
    source_peer: &PeerId,
    label: &Arc<str>,
) {
    let observer = inner.observer.read().clone();
    if let Some(obs) = observer {
        obs.on_event(RoomEvent::DataChannelOpened {
            room: room_id.clone(),
            peer: source_peer.clone(),
            label: label.to_string(),
        })
        .await;
    }
}

async fn attach_dc_destinations_to_existing_members(
    inner: &Arc<SfuInner>,
    room: &Arc<crate::room::RoomState>,
    source_peer: &PeerId,
    forwarder: &Arc<DataChannelForwarder>,
) {
    let members: Vec<PeerId> = room
        .members
        .iter()
        .map(|m| m.clone())
        .filter(|m| m != source_peer)
        .collect();

    for member in members {
        let Some(entry) = inner.peers.get(&member) else {
            continue;
        };
        let entry = Arc::clone(entry.value());
        let pc_opt = entry.peer_connection.lock().await.clone();
        let Some(other_pc) = pc_opt else { continue };

        let init = RTCDataChannelInit {
            ordered: Some(forwarder.ordered),
            max_packet_life_time: forwarder.max_packet_life_time,
            max_retransmits: forwarder.max_retransmits,
            protocol: Some(forwarder.protocol.to_string()),
            ..Default::default()
        };

        let local_dc = match other_pc
            .create_data_channel(forwarder.label.as_ref(), Some(init))
            .await
        {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "create_data_channel for {} (label={}) failed: {:?}",
                    member, forwarder.label, e
                );
                continue;
            }
        };

        forwarder
            .destination_channels
            .insert(member.clone(), Arc::clone(&local_dc));

        // Renegotiation is required for the new SCTP m-line on existing PCs.
        let other_pc = Arc::clone(&other_pc);
        let sink = Arc::clone(&entry.sink);
        let member_id = member.clone();
        tokio::spawn(async move {
            spawn_renegotiation_offer(other_pc, sink, member_id).await;
        });
    }
}

fn install_message_forwarder(
    inner: Arc<SfuInner>,
    forwarder: Arc<DataChannelForwarder>,
    dc: Arc<RTCDataChannel>,
    source_id: DataChannelSourceId,
) {
    // Capture the `on_close` event to clean up the room state and notify
    // the host. Cheap: cloning Arcs only.
    {
        let inner_close = Arc::clone(&inner);
        let source_peer_close = forwarder.source_peer.clone();
        let label_close = Arc::clone(&forwarder.label);
        let source_id_close = source_id.clone();
        dc.on_close(Box::new(move || {
            let inner = Arc::clone(&inner_close);
            let source_peer = source_peer_close.clone();
            let label = Arc::clone(&label_close);
            let source_id = source_id_close.clone();
            Box::pin(async move {
                cleanup_data_channel(&inner, &source_peer, &label, &source_id).await;
            })
        }));
    }

    let label = Arc::clone(&forwarder.label);
    let dest_channels = Arc::clone(&forwarder.destination_channels);
    let source_id_msg = source_id.clone();
    dc.on_message(Box::new(move |msg| {
        let inner = Arc::clone(&inner);
        let label = Arc::clone(&label);
        let dest_channels = Arc::clone(&dest_channels);
        let source_id = source_id_msg.clone();
        Box::pin(async move {
            forward_message(
                inner,
                source_id,
                label,
                dest_channels,
                msg.is_string,
                msg.data,
            )
            .await;
        })
    }));
}

async fn forward_message(
    inner: Arc<SfuInner>,
    source_id: DataChannelSourceId,
    label: Arc<str>,
    dest_channels: Arc<DashMap<PeerId, Arc<RTCDataChannel>>>,
    is_string: bool,
    data: Bytes,
) {
    // ---- Ingress interceptor pass ----
    let mut payload = data;
    let mut payload_is_string = is_string;
    if !inner.config.dc_interceptors.is_empty()
        && run_dc_ingress(
            &inner.config.dc_interceptors,
            &source_id,
            &label,
            &mut payload_is_string,
            &mut payload,
        )
        .await
    {
        return;
    }

    if dest_channels.is_empty() {
        return;
    }

    // ---- Per-destination egress + write ----
    for entry in dest_channels.iter() {
        let dest_user = entry.key().clone();
        let dest_dc = Arc::clone(entry.value());

        let mut egress_payload = payload.clone();
        let mut egress_is_string = payload_is_string;
        if !inner.config.dc_interceptors.is_empty()
            && run_dc_egress(
                &inner.config.dc_interceptors,
                &source_id,
                &label,
                &dest_user,
                &mut egress_is_string,
                &mut egress_payload,
            )
            .await
        {
            continue;
        }

        let send_result = if egress_is_string {
            match std::str::from_utf8(&egress_payload) {
                Ok(s) => dest_dc.send_text(s).await.map(|_| ()),
                Err(_) => {
                    debug!(
                        "dc string payload not valid UTF-8; dropping for {}",
                        dest_user
                    );
                    continue;
                }
            }
        } else {
            dest_dc.send(&egress_payload).await.map(|_| ())
        };
        if let Err(e) = send_result {
            debug!("dc send to {} failed: {:?}", dest_user, e);
        }
    }
}

async fn cleanup_data_channel(
    inner: &Arc<SfuInner>,
    source_peer: &PeerId,
    label: &Arc<str>,
    source_id: &DataChannelSourceId,
) {
    // Remove the forwarder from any room that holds it; in practice only
    // one room can â€” peers are mono-room today (P4 will lift this).
    let mut affected_room: Option<RoomId> = None;
    for kv in inner.rooms.iter() {
        if kv.value().dc_forwarders.remove(source_id).is_some() {
            affected_room = Some(kv.key().clone());
            break;
        }
    }
    if let Some(room_id) = affected_room {
        let observer = inner.observer.read().clone();
        if let Some(obs) = observer {
            obs.on_event(RoomEvent::DataChannelClosed {
                room: room_id,
                peer: source_peer.clone(),
                label: label.to_string(),
            })
            .await;
        }
    }
}
