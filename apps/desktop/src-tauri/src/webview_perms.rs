// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// SPDX-License-Identifier: BUSL-1.1

//! WebView2 permission auto-grant.
//!
//! On Windows, the embedded WebView2 (Chromium-based) intercepts every
//! permission request issued by the renderer (mic, camera, screen capture,
//! etc.) and — without an explicit handler — defers to a host UI prompt that
//! Tauri does not provide. The result is a silent denial: every
//! `getUserMedia({ audio: true })` rejects with `NotAllowedError`, the voice
//! pipeline never acquires a track and the user appears alone in the channel
//! (`sfu_active_peers=0`).
//!
//! The previous mitigation relied on the
//! `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` environment variable
//! (`--use-fake-ui-for-media-stream`) but this is fragile: it is read once
//! when the WebView2 user-data folder is initialized and ignored on
//! subsequent starts if the runtime cached a different configuration.
//!
//! This module subscribes directly to `ICoreWebView2::add_PermissionRequested`
//! through the COM API, which fires for every renderer-initiated permission
//! request and is honoured regardless of any cached state.

#[cfg(target_os = "windows")]
use tauri::WebviewWindow;

#[cfg(target_os = "windows")]
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_2, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    },
    PermissionRequestedEventHandler,
};

#[cfg(target_os = "windows")]
use windows::core::Interface;

/// Installs an auto-allow `PermissionRequested` handler on the main WebView2
/// instance for microphone and camera capture. Any other permission kind
/// (geolocation, notifications, etc.) is left to its default behaviour.
///
/// Errors from the underlying COM layer are logged and swallowed: a missing
/// handler degrades gracefully to the previous (env-flag) behaviour rather
/// than aborting startup.
#[cfg(target_os = "windows")]
pub fn install_media_auto_grant(window: &WebviewWindow) {
    let _ = window.with_webview(|webview| {
        // Safety: the controller and core webview pointers are valid for the
        // lifetime of the window; the handler closure captures no Tauri state.
        unsafe {
            let controller = webview.controller();
            let core = match controller.CoreWebView2() {
                Ok(c) => c,
                Err(err) => {
                    log::warn!("[webview_perms] CoreWebView2() failed: {err:?}");
                    return;
                }
            };

            // `add_PermissionRequested` lives on the base ICoreWebView2; cast
            // through the v2 interface to match the webview2-com bindings.
            let core_v2: ICoreWebView2_2 = match core.cast() {
                Ok(c) => c,
                Err(err) => {
                    log::warn!("[webview_perms] ICoreWebView2_2 cast failed: {err:?}");
                    return;
                }
            };

            let handler = PermissionRequestedEventHandler::create(Box::new(|_sender, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = Default::default();
                args.PermissionKind(&mut kind)?;
                if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                    || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA
                {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                }
                Ok(())
            }));

            let mut token = Default::default();
            if let Err(err) = core_v2.add_PermissionRequested(&handler, &mut token) {
                log::warn!("[webview_perms] add_PermissionRequested failed: {err:?}");
            } else {
                log::info!("[webview_perms] mic/camera auto-grant handler installed");
            }
        }
    });
}

/// No-op on non-Windows platforms — macOS and Linux delegate permission
/// management to the OS (TCC / pipewire portals) which is handled by the
/// bundle entitlements.
#[cfg(not(target_os = "windows"))]
pub fn install_media_auto_grant<W>(_window: &W) {}
