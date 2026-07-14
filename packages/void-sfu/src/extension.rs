// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Public extension points: traits hosts (and future WASM plugins) implement
//! to observe or alter the SFU's behavior without forking the crate.
//!
//! All traits provide no-op default implementations so adding a new method
//! later is non-breaking. The fast path (`PacketInterceptor::on_rtp`) is
//! invoked once per packet and once per destination, so implementations
//! must stay allocation-light.

use async_trait::async_trait;
use bytes::Bytes;
use webrtc::rtp::packet::Packet;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;

use crate::id::{DataChannelSourceId, MediaSourceId, PeerId};

/// Direction of a packet relative to the SFU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Source publisher â†’ SFU. The interceptor runs **once** per packet,
    /// before fan-out, with `destination = None`.
    Ingress,
    /// SFU â†’ destination subscriber. The interceptor runs **once per
    /// destination** with `destination = Some(peer)`.
    Egress,
}

/// Context handed to [`PacketInterceptor::on_rtp`].
///
/// References borrow from the forwarding worker's locals so no clone is
/// performed unless the implementation explicitly clones a field.
pub struct PacketContext<'a> {
    pub source: &'a MediaSourceId,
    pub destination: Option<&'a PeerId>,
    /// Media kind hint (`"audio"`, `"video"`, …) as advertised by the
    /// publishing track.
    pub kind: &'a str,
    pub direction: Direction,
}

/// Outcome of an interceptor invocation.
#[derive(Debug, Clone)]
pub enum InterceptOutcome {
    /// Forward the packet unchanged.
    Forward,
    /// Drop this packet on this leg only (other interceptors and other
    /// destinations are unaffected for ingress; the current destination
    /// is skipped for egress).
    Drop,
    /// Replace the payload/header with the supplied packet before further
    /// processing. Subsequent interceptors operate on the replacement.
    Replace(Packet),
}

/// RTP-level interceptor.
///
/// Stateless instances are encouraged; if state is needed, prefer
/// `dashmap`/`parking_lot` to avoid blocking the hot path.
#[async_trait]
pub trait PacketInterceptor: Send + Sync + 'static {
    /// Called once per packet on ingress and once per destination on egress.
    /// Default behavior is to forward the packet unchanged.
    async fn on_rtp(&self, ctx: PacketContext<'_>, packet: &Packet) -> InterceptOutcome {
        let _ = (ctx, packet);
        InterceptOutcome::Forward
    }
}

/// Codec selection policy.
///
/// Hosts implement this to constrain which codecs the SFU accepts at
/// negotiation time (e.g. enforce a single video codec to bound CPU cost,
/// or restrict audio to a specific sample rate). Returning `false`
/// excludes the codec from the SFU's published capabilities.
pub trait CodecPolicy: Send + Sync + 'static {
    /// Whether the supplied codec capability should be advertised for the
    /// given media kind (`"audio"` / `"video"`). Defaults to `true`.
    fn allow(&self, kind: &str, codec: &RTCRtpCodecCapability) -> bool {
        let _ = (kind, codec);
        true
    }
}

/// Context handed to [`DataChannelInterceptor::on_message`].
pub struct DataChannelContext<'a> {
    pub source: &'a DataChannelSourceId,
    pub destination: Option<&'a PeerId>,
    pub label: &'a str,
    /// Whether the original payload was sent as a UTF-8 string.
    pub is_string: bool,
    pub direction: Direction,
}

/// Outcome of a data channel interceptor invocation.
#[derive(Debug, Clone)]
pub enum DataChannelOutcome {
    /// Forward the message unchanged.
    Forward,
    /// Drop this message on this leg only.
    Drop,
    /// Replace the payload before further processing. Subsequent
    /// interceptors operate on the replacement.
    Replace { is_string: bool, data: Bytes },
}

/// SCTP data channel interceptor.
///
/// Same hot-path contract as [`PacketInterceptor`]: invoked once per message
/// on ingress and once per destination on egress. Stateless implementations
/// are encouraged.
#[async_trait]
pub trait DataChannelInterceptor: Send + Sync + 'static {
    async fn on_message(
        &self,
        ctx: DataChannelContext<'_>,
        is_string: bool,
        data: &Bytes,
    ) -> DataChannelOutcome {
        let _ = (ctx, is_string, data);
        DataChannelOutcome::Forward
    }
}
