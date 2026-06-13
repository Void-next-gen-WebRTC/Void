// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! SDP-level entry points (offer / answer / ICE) and PC bootstrap helpers.

use std::sync::Arc;

use tracing::{debug, warn};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::error::{SfuError, SfuResult};
use crate::forwarder::PeerEntry;
use crate::id::PeerId;
use crate::models::IceCandidate;
use crate::sfu::SfuInner;
use crate::signal::Outbound;

use super::catchup::catchup_existing_tracks;
use super::data_channel::install_on_data_channel;
use super::tracks::install_on_track;

/// Builds the [`RTCConfiguration`] for a new PC from the SFU config.
pub(super) fn build_rtc_config(ice_servers: &[String]) -> RTCConfiguration {
    // Si PUBLIC_IP est définie, on est sur le cloud (Oracle).
    // Le SFU n'a pas besoin de STUN pour lui-même car on utilise set_nat_1to1_ips.
    let final_ice_servers = if std::env::var("PUBLIC_IP").is_ok() {
        println!("ℹ️ [SFU-CONFIG] IP publique détectée : désactivation des serveurs STUN/TURN pour le SFU.");
        vec![] // On laisse la liste VIDE pour le serveur
    } else {
        // En mode local/LAN, on garde la config d'origine
        vec![RTCIceServer {
            urls: ice_servers.to_vec(),
            ..Default::default()
        }]
    };

    RTCConfiguration {
        ice_servers: final_ice_servers,
        ..Default::default()
    }
}

/// Handles an incoming SDP offer for `peer_id`.
///
/// Two paths:
/// * **Bootstrap** (no PC yet): build the PC, install `on_ice_candidate`,
///   `on_track`, `on_data_channel`, set the remote offer, answer, persist
///   the PC, deliver the answer, and run `catchup_existing_tracks` to add
///   already-published forwarders.
/// * **Renegotiation** (PC already exists): reuse the live PC and just
///   apply `set_remote_description(offer)` → `create_answer` →
///   `set_local_description(answer)` → deliver. This is critical: the
///   client renegotiates whenever it adds/removes a track (mute toggle,
///   screen-share start/stop). Tearing down the PC each time would drop
///   every forwarder previously plugged in by `catchup_existing_tracks`,
///   silently isolating peers mid-call.
///
/// The answer is always sent before any catch-up to unblock polite clients.
pub(crate) async fn handle_offer(
    inner: Arc<SfuInner>,
    peer_id: PeerId,
    sdp: &str,
) -> SfuResult<()> {
    let sdp_str = sdp.to_string();

    // Resolve the peer's current room (must be set by `join_room` first).
    let peer_entry = inner
        .peers
        .get(&peer_id)
        .ok_or_else(|| SfuError::PeerNotFound(peer_id.as_arc()))?
        .value()
        .clone();

    let room_id = peer_entry
        .room
        .read()
        .clone()
        .ok_or_else(|| SfuError::PeerNotInRoom(peer_id.as_arc()))?;

    // ---- Renegotiation fast-path: reuse the existing PC ----
    {
        let existing = peer_entry.peer_connection.lock().await.clone();
        if let Some(pc) = existing {
            let session = RTCSessionDescription::offer(sdp_str)
                .map_err(|e| SfuError::InvalidSdp(e.to_string()))?;
            pc.set_remote_description(session).await?;
            let answer = pc.create_answer(None).await?;
            pc.set_local_description(answer.clone()).await?;
            let answer_sdp = answer.sdp.clone();
            if let Err(e) = peer_entry
                .sink
                .deliver(&peer_id, Outbound::Answer { sdp: answer_sdp })
                .await
            {
                warn!(
                    "sink delivery (answer/reneg) failed for {}: {:?}",
                    peer_id, e
                );
            }
            return Ok(());
        }
    }

    // ---- Bootstrap path: build a fresh PC ----
    let pc = Arc::new(
        inner
            .api
            .new_peer_connection(build_rtc_config(&inner.config.ice_servers))
            .await?,
    );

    install_ice_relay(&pc, &peer_entry);
    install_on_track(&pc, Arc::clone(&inner), peer_id.clone(), room_id.clone());
    install_on_data_channel(&pc, Arc::clone(&inner), peer_id.clone(), room_id.clone());

    let session =
        RTCSessionDescription::offer(sdp_str).map_err(|e| SfuError::InvalidSdp(e.to_string()))?;
    pc.set_remote_description(session).await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;

    // Persist the PC on the peer entry *before* sending the answer so any
    // concurrent `handle_ice` finds it.
    {
        let mut guard = peer_entry.peer_connection.lock().await;
        *guard = Some(Arc::clone(&pc));
    }

    let answer_sdp = answer.sdp.clone();
    if let Err(e) = peer_entry
        .sink
        .deliver(&peer_id, Outbound::Answer { sdp: answer_sdp })
        .await
    {
        warn!("sink delivery (answer) failed for {}: {:?}", peer_id, e);
    }

    // ---- Catchup runs AFTER the answer is on the wire (race fix) ----
    catchup_existing_tracks(inner, peer_id, room_id, pc).await
}

/// Wires `on_ice_candidate` to forward server-side candidates back to the peer.
pub(super) fn install_ice_relay(
    pc: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    peer_entry: &Arc<PeerEntry>,
) {
    let sink = Arc::clone(&peer_entry.sink);
    let pid = peer_entry.id.clone();
    pc.on_ice_candidate(Box::new(move |c| {
        let sink = Arc::clone(&sink);
        let pid = pid.clone();
        Box::pin(async move {
            let Some(candidate) = c else { return };
            let init = match candidate.to_json() {
                Ok(j) => j,
                Err(e) => {
                    warn!("ice to_json failed: {:?}", e);
                    return;
                }
            };
            let ice = IceCandidate {
                candidate: init.candidate,
                sdp_mid: init.sdp_mid,
                sdp_mline_index: init.sdp_mline_index,
                username_fragment: init.username_fragment,
            };
            if let Err(e) = sink.deliver(&pid, Outbound::Ice { candidate: ice }).await {
                debug!("sink ice delivery failed: {:?}", e);
            }
        })
    }));
}

/// Applies an SDP answer to the peer's existing PC.
pub(crate) async fn handle_answer(
    inner: Arc<SfuInner>,
    peer_id: PeerId,
    sdp: &str,
) -> SfuResult<()> {
    let sdp_str = sdp.to_string();

    let entry = inner
        .peers
        .get(&peer_id)
        .ok_or_else(|| SfuError::PeerNotFound(peer_id.as_arc()))?
        .value()
        .clone();

    let pc_opt = entry.peer_connection.lock().await.clone();
    let pc = pc_opt.ok_or(SfuError::Internal("answer received before offer"))?;
    let session =
        RTCSessionDescription::answer(sdp_str).map_err(|e| SfuError::InvalidSdp(e.to_string()))?;
    pc.set_remote_description(session).await?;
    Ok(())
}

/// Adds an ICE candidate to the peer's PC.
pub(crate) async fn handle_ice(
    inner: Arc<SfuInner>,
    peer_id: PeerId,
    candidate: IceCandidate,
) -> SfuResult<()> {
    let entry = inner
        .peers
        .get(&peer_id)
        .ok_or_else(|| SfuError::PeerNotFound(peer_id.as_arc()))?
        .value()
        .clone();

    let pc_opt = entry.peer_connection.lock().await.clone();
    let Some(pc) = pc_opt else {
        // ICE before offer is benign; clients buffer their own candidates.
        return Ok(());
    };

    let init = RTCIceCandidateInit {
        candidate: candidate.candidate,
        sdp_mid: candidate.sdp_mid,
        sdp_mline_index: candidate.sdp_mline_index,
        username_fragment: candidate.username_fragment,
    };
    pc.add_ice_candidate(init).await?;
    Ok(())
}
