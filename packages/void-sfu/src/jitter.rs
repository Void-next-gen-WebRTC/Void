// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

use std::collections::VecDeque;

use webrtc::rtp::packet::Packet;

/// Adaptive jitter buffer that drops packets older than `playout_delay_ms`.
///
/// The buffer takes ownership of incoming packets but the underlying RTP
/// payload is `bytes::Bytes` (Arc-backed) so storage and removal are O(1)
/// without copying media data.
pub struct JitterBuffer {
    packets: VecDeque<Packet>,
    playout_delay_ms: u32,
    clock_rate: u32,
    last_timestamp: u32,
}

impl JitterBuffer {
    /// Creates a buffer pre-allocated for ~100 packets (~2 s of Opus @ 20 ms).
    #[inline]
    pub fn new(playout_delay_ms: u32, clock_rate: u32) -> Self {
        Self {
            packets: VecDeque::with_capacity(100),
            playout_delay_ms,
            clock_rate,
            last_timestamp: 0,
        }
    }

    /// Appends a packet and prunes entries older than the playout window.
    pub fn push(&mut self, packet: Packet) {
        let timestamp = packet.header.timestamp;
        self.packets.push_back(packet);
        self.last_timestamp = timestamp;

        let playout = self.playout_delay_ms;
        let clock = self.clock_rate;
        let last_t = self.last_timestamp;

        self.packets.retain(|p| {
            let diff = last_t.wrapping_sub(p.header.timestamp);
            // Saturate to avoid div-by-zero when configured with clock=0.
            let age_ms = diff
                .checked_mul(1000)
                .and_then(|v| v.checked_div(clock))
                .unwrap_or(0);
            age_ms <= playout
        });
    }

    /// Returns the next packet that has reached its playout deadline.
    pub fn pop(&mut self) -> Option<Packet> {
        let front = self.packets.front()?;
        let age_ms = self.calculate_age_ms(front.header.timestamp);
        if age_ms >= self.playout_delay_ms {
            return self.packets.pop_front();
        }
        None
    }

    fn calculate_age_ms(&self, timestamp: u32) -> u32 {
        if self.last_timestamp == 0 || self.clock_rate == 0 {
            return 0;
        }
        let diff = self.last_timestamp.wrapping_sub(timestamp);
        diff * 1000 / self.clock_rate
    }
}
