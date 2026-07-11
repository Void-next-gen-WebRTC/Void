// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
//! Sanity checks for the host-side `AppState` constants.
//!
//! Note: `JitterBuffer`, `RTCPStats` and the RTP channel capacity were
//! moved out of this crate when the SFU was extracted to `void-sfu`.
//! Those types are now tested in-tree inside that library (see
//! `packages/void-sfu/src/jitter.rs` and `packages/void-sfu/tests/`).
#[test]
fn channel_capacity_constants_are_positive() {
    use crate::sfu::state::{CHAT_HISTORY_CAP, WS_CHANNEL_CAPACITY};
    const {
        assert!(WS_CHANNEL_CAPACITY > 0);
        assert!(CHAT_HISTORY_CAP > 0);
    }
}
