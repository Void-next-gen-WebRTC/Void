// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

use std::sync::Arc;
use thiserror::Error;
use webrtc::Error as WebRtcError;

/// Errors returned by the SFU. Every public API surfaces these instead of panicking.
#[derive(Debug, Error)]
pub enum SfuError {
    /// The peer id is unknown to the SFU.
    #[error("peer not found: {0}")]
    PeerNotFound(Arc<str>),

    /// The peer already exists.
    #[error("peer already exists: {0}")]
    PeerAlreadyExists(Arc<str>),

    /// The peer is not currently in any room.
    #[error("peer is not joined to a room: {0}")]
    PeerNotInRoom(Arc<str>),

    /// SDP parsing or session description build failure.
    #[error("invalid SDP: {0}")]
    InvalidSdp(String),

    /// ICE candidate parse failure.
    #[error("invalid ICE candidate: {0}")]
    InvalidIce(String),

    /// Underlying webrtc-rs error.
    #[error("webrtc error: {0}")]
    WebRtc(#[from] WebRtcError),

    /// The signaling sink (consumer) reported a delivery failure.
    #[error("signal sink delivery failed for peer {peer}")]
    SinkDelivery { peer: Arc<str> },

    /// Generic internal invariant violation. Never `panic!()` — bubble up instead.
    #[error("internal invariant violated: {0}")]
    Internal(&'static str),
}

/// Convenient `Result` alias.
pub type SfuResult<T> = Result<T, SfuError>;
