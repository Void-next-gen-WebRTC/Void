// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! # signaling-server (library facade)
//!
//! This crate is built as both a binary (the actual signaling daemon — see
//! `src/main.rs`) and a library so that integration tests, criterion
//! benchmarks and any future tooling can import the internal modules
//! without going through HTTP/WebSocket.
//!
//! Every module re-exported here is part of the **internal** surface of
//! the daemon. The wire-level public contract is the HTTP + WS API
//! described in `SPEC.md` — the Rust items below are subject to change.

pub mod auth;
pub mod errors;
pub mod fraud;
pub mod friends;
pub mod gateway;
pub mod metrics;
pub mod models;
pub mod negotiate;
pub mod nonce;
pub mod store;
pub mod turn;

#[cfg(test)]
mod tests;
