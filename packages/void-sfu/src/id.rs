// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

use std::fmt;
use std::sync::Arc;

/// Cheap-clone string newtype for peer identifiers.
///
/// Backed by [`Arc<str>`] so it is cloned by reference-counting only â€” no heap
/// copy. Comparison and hashing delegate to the underlying string slice.
#[derive(Clone, Eq)]
pub struct PeerId(Arc<str>);

impl PeerId {
    /// Creates a new id from any string-like input.
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    /// Borrows the inner string slice without allocating.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns a clone of the underlying [`Arc<str>`].
    #[inline]
    pub fn as_arc(&self) -> Arc<str> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerId({})", self.0)
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq for PeerId {
    fn eq(&self, other: &Self) -> bool {
        // Compare by content (handles different Arc instances pointing to equal strings).
        self.0.as_ref() == other.0.as_ref()
    }
}

impl std::hash::Hash for PeerId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_ref().hash(state);
    }
}

impl From<&str> for PeerId {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl From<String> for PeerId {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl AsRef<str> for PeerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Cheap-clone string newtype for room identifiers. Same semantics as [`PeerId`].
#[derive(Clone, Eq)]
pub struct RoomId(Arc<str>);

impl RoomId {
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn as_arc(&self) -> Arc<str> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RoomId({})", self.0)
    }
}

impl fmt::Display for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq for RoomId {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ref() == other.0.as_ref()
    }
}

impl std::hash::Hash for RoomId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_ref().hash(state);
    }
}

impl From<&str> for RoomId {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl From<String> for RoomId {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl AsRef<str> for RoomId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Unique identifier of a media source (one published RTP track) within the
/// SFU. Built as `format!("{peer_id}:{rtp_track_id}")` so it stays unique
/// across peers but the host can derive it from a `(PeerId, track_id)`
/// pair without a registry lookup.
///
/// Same cheap-clone semantics as [`PeerId`]/[`RoomId`].
#[derive(Clone, Eq)]
pub struct MediaSourceId(Arc<str>);

impl MediaSourceId {
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    /// Builds a deterministic id from a peer id and the underlying RTP
    /// track id (the `id` value of the `MediaStreamTrack` on the publisher).
    pub fn from_peer_and_track(peer: &PeerId, track_id: &str) -> Self {
        let mut s = String::with_capacity(peer.as_str().len() + 1 + track_id.len());
        s.push_str(peer.as_str());
        s.push(':');
        s.push_str(track_id);
        Self(Arc::from(s))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn as_arc(&self) -> Arc<str> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for MediaSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MediaSourceId({})", self.0)
    }
}

impl fmt::Display for MediaSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq for MediaSourceId {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ref() == other.0.as_ref()
    }
}

impl std::hash::Hash for MediaSourceId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_ref().hash(state);
    }
}

impl From<&str> for MediaSourceId {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl From<String> for MediaSourceId {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl AsRef<str> for MediaSourceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Unique identifier of a published data channel within the SFU.
/// Built as `format!("{peer_id}:dc:{label}")` so colliding labels across
/// peers stay distinguishable.
#[derive(Clone, Eq)]
pub struct DataChannelSourceId(Arc<str>);

impl DataChannelSourceId {
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    /// Builds a deterministic id from a peer id and the channel label.
    pub fn from_peer_and_label(peer: &PeerId, label: &str) -> Self {
        let mut s = String::with_capacity(peer.as_str().len() + 4 + label.len());
        s.push_str(peer.as_str());
        s.push_str(":dc:");
        s.push_str(label);
        Self(Arc::from(s))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn as_arc(&self) -> Arc<str> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for DataChannelSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DataChannelSourceId({})", self.0)
    }
}

impl fmt::Display for DataChannelSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq for DataChannelSourceId {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ref() == other.0.as_ref()
    }
}

impl std::hash::Hash for DataChannelSourceId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_ref().hash(state);
    }
}

impl From<&str> for DataChannelSourceId {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl From<String> for DataChannelSourceId {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl AsRef<str> for DataChannelSourceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
