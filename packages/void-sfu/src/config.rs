// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::extension::{CodecPolicy, DataChannelInterceptor, PacketInterceptor};

/// Per-codec jitter buffer override. The `clock_rate` defaults to the value
/// advertised by the codec capability (e.g. 48000 for Opus, 90000 for
/// VP8/VP9/H264/AV1) so most hosts only need to tweak `playout_ms`.
#[derive(Debug, Clone)]
pub struct JitterPolicy {
    pub playout_ms: u32,
    pub clock_rate: Option<u32>,
}

/// Static configuration for an [`crate::Sfu`] instance.
///
/// Defaults are sensible for low-latency real-time media. The jitter clock
/// rate is no longer global: it is derived per-track from the negotiated
/// codec capability, and overrides can be supplied per MIME type via
/// [`SfuConfig::jitter_overrides`].
#[derive(Clone)]
pub struct SfuConfig {
    /// ICE servers exposed in every PeerConnection configuration.
    pub ice_servers: Vec<String>,

    /// Default jitter buffer playout delay in milliseconds.
    pub jitter_playout_ms: u32,

    /// Optional per-MIME-type jitter overrides (e.g. `"audio/opus"`,
    /// `"video/VP8"`). Lookup is case-insensitive on the MIME type.
    pub jitter_overrides: HashMap<String, JitterPolicy>,

    /// Bounded RTP forwarding channel capacity per source track.
    pub rtp_channel_capacity: usize,

    /// Interval between batched bandwidth-stats flushes.
    pub stats_flush_interval: Duration,

    /// Ordered chain of RTP interceptors. Invoked on ingress (once per
    /// packet) and on egress (once per destination). Empty by default —
    /// the hot path short-circuits when this list is empty so there is
    /// no per-packet cost unless an interceptor is registered.
    pub interceptors: Vec<Arc<dyn PacketInterceptor>>,

    /// Ordered chain of data-channel interceptors. Same hot-path contract
    /// as [`SfuConfig::interceptors`], applied to SCTP messages.
    pub dc_interceptors: Vec<Arc<dyn DataChannelInterceptor>>,

    /// Optional codec admission policy. `None` accepts every codec the
    /// underlying webrtc-rs media engine supports (default behavior).
    pub codec_policy: Option<Arc<dyn CodecPolicy>>,
}

impl std::fmt::Debug for SfuConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SfuConfig")
            .field("ice_servers", &self.ice_servers)
            .field("jitter_playout_ms", &self.jitter_playout_ms)
            .field("jitter_overrides", &self.jitter_overrides)
            .field("rtp_channel_capacity", &self.rtp_channel_capacity)
            .field("stats_flush_interval", &self.stats_flush_interval)
            .field(
                "interceptors",
                &format_args!("[{} registered]", self.interceptors.len()),
            )
            .field(
                "dc_interceptors",
                &format_args!("[{} registered]", self.dc_interceptors.len()),
            )
            .field(
                "codec_policy",
                &self.codec_policy.as_ref().map(|_| "<dyn CodecPolicy>"),
            )
            .finish()
    }
}

impl Default for SfuConfig {
    fn default() -> Self {
        Self {
            ice_servers: vec!["stun:stun.l.google.com:19302".to_string()],
            jitter_playout_ms: 30,
            jitter_overrides: HashMap::new(),
            rtp_channel_capacity: 500,
            stats_flush_interval: Duration::from_millis(500),
            interceptors: Vec::new(),
            dc_interceptors: Vec::new(),
            codec_policy: None,
        }
    }
}
