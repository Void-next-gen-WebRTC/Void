// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! RTCP feedback API.
//!
//! For now this module exposes a single high-level operation —
//! [`Sfu::request_keyframe`] — synthesising a `PictureLossIndication`
//! addressed at a given media source. Future extensions (FIR, NACK relay,
//! TWCC bandwidth estimation) plug in here.

use std::sync::Arc;

use tracing::warn;
use webrtc::rtcp::packet::Packet as RtcpPacket;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;

use crate::error::{SfuError, SfuResult};
use crate::id::MediaSourceId;
use crate::sfu::Sfu;

impl Sfu {
    /// Requests a keyframe from the publisher of `media_source_id` by
    /// emitting a Picture Loss Indication on its PeerConnection.
    ///
    /// This is the standard mechanism subscribers use after a freeze
    /// (decoder reset, layer switch, late join) to force the publisher's
    /// encoder to emit an IDR/keyframe.
    ///
    /// Returns `SfuError::PeerNotFound` if the source is unknown or its
    /// PC has already been torn down.
    pub async fn request_keyframe(&self, media_source_id: &MediaSourceId) -> SfuResult<()> {
        // Locate the forwarder by scanning rooms; the lookup is cheap (one
        // shard load per room) and the call frequency is low (PLI bursts
        // are rate-limited host-side anyway).
        let mut found: Option<(crate::id::PeerId, u32)> = None;
        for kv in self.inner.rooms.iter() {
            if let Some(forwarder) = kv.value().forwarders.get(media_source_id) {
                let f = forwarder.value();
                found = Some((f.source_peer.clone(), f.ssrc));
                break;
            }
        }
        let (source_peer, media_ssrc) =
            found.ok_or_else(|| SfuError::PeerNotFound(Arc::from(media_source_id.as_str())))?;

        let entry = self
            .inner
            .peers
            .get(&source_peer)
            .ok_or_else(|| SfuError::PeerNotFound(source_peer.as_arc()))?
            .value()
            .clone();

        let pc_opt = entry.peer_connection.lock().await.clone();
        let pc = pc_opt.ok_or(SfuError::Internal(
            "publisher PC missing while requesting keyframe",
        ))?;

        let pli = PictureLossIndication {
            sender_ssrc: 0,
            media_ssrc,
        };
        if let Err(e) = pc
            .write_rtcp(&[Box::new(pli) as Box<dyn RtcpPacket + Send + Sync>])
            .await
        {
            warn!(
                "PLI write failed for source={} ssrc={}: {:?}",
                media_source_id, media_ssrc, e
            );
            return Err(SfuError::WebRtc(e));
        }
        Ok(())
    }
}
