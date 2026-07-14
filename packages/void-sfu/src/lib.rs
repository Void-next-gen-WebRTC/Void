// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! # void-sfu
//!
//! A domain-agnostic Selective Forwarding Unit (SFU) library built on top
//! of the [`webrtc`] crate (a.k.a. *webrtc-rs*).
//!
//! ## Design goals
//! - **Agnostic.** No coupling to any application-specific concept. The
//!   library only speaks WebRTC; the host decides how to ferry signaling
//!   messages, what identifiers mean, and how peers/rooms map to its
//!   own domain model.
//! - **Zero-copy on the data path.** RTP packets are forwarded by reference
//!   (`bytes::Bytes` is `Arc`-backed); identifiers are wrapped in `Arc<str>`
//!   newtypes so cloning is a refcount bump, not a heap copy.
//! - **No panics.** Every fallible operation returns `Result<_, SfuError>`.
//!   The library never calls `unwrap`/`expect`/`panic!` on input from the
//!   network or from host calls.
//! - **Hot-pluggable extensions.** Hosts (and, in a future iteration,
//!   sandboxed WASM modules) can attach packet interceptors and codec
//!   policies at runtime through opaque trait objects, without forking
//!   or restarting the SFU.
//!
//! ## Quick start
//! ```ignore
//! use std::sync::Arc;
//! use void_sfu::{Sfu, SfuConfig, PeerId, RoomId, SignalSink, Outbound, SfuResult};
//! use async_trait::async_trait;
//!
//! struct MySink { /* … your transport … */ }
//!
//! #[async_trait]
//! impl SignalSink for MySink {
//!     async fn deliver(&self, peer: &PeerId, message: Outbound) -> SfuResult<()> {
//!         /* serialize `message` and push it to your transport */
//!         Ok(())
//!     }
//! }
//!
//! # async fn run() -> SfuResult<()> {
//! let sfu = Sfu::new(SfuConfig::default())?;
//! let alice = PeerId::from("alice");
//! sfu.add_peer(alice.clone(), Arc::new(MySink {  }))?;
//! sfu.join_room(&alice, RoomId::from("room-1")).await?;
//! # Ok(()) }
//! ```

mod config;
mod dc_forwarder;
mod error;
mod extension;
mod forwarder;
mod id;
mod jitter;
mod metrics;
mod models;
mod negotiation;
mod room;
mod rtcp;
mod sfu;
mod signal;
mod stats;

pub use config::{JitterPolicy, SfuConfig};
pub use error::{SfuError, SfuResult};
pub use extension::{
    CodecPolicy, DataChannelContext, DataChannelInterceptor, DataChannelOutcome, Direction,
    InterceptOutcome, PacketContext, PacketInterceptor,
};
pub use id::{DataChannelSourceId, MediaSourceId, PeerId, RoomId};
pub use metrics::MetricsSnapshot;
pub use models::{IceCandidate, MediaKind};
pub use room::RoomPeer;
pub use sfu::{JoinSnapshot, Sfu};
pub use signal::{Outbound, RoomEvent, RoomObserver, SignalSink};
pub use stats::ForwardingStats;

/// Bench-only re-exports. Enabled with `--features bench`.
/// Never use these in production code; they expose internal types whose
/// API is not covered by semver.
#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod __bench_jitter {
    pub use crate::jitter::JitterBuffer;
}
