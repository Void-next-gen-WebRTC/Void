pub mod identity;
pub mod layout;
pub mod tls;
pub mod webview_perms;

use std::sync::Arc;

use tauri::{Emitter, Listener, Manager};

use layout::{
    default_layout, handle_move, handle_resize, handle_swap, is_legacy_pixel_layout,
    load_layout_from_disk, LayoutBatch, LayoutState, MovePayload, ResizePayload, SwapPayload,
};

// ---------------------------------------------------------------------------
// DSP runtime seal
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_dsp_token() -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(b"v0id-rt-seal");
    h.update(&0x564F_4944u32.to_le_bytes());
    h.update(&0x5253_4543u32.to_le_bytes());
    h.finalize()
}

// ---------------------------------------------------------------------------
// Application entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Auto-grant microphone / camera / screen-capture permissions inside the
    // embedded WebView2. Without this, the very first call to
    // `getUserMedia({ audio: true })` from the renderer triggers a Windows
    // permission prompt that — in WebView2 builds without a host UI for
    // permission dialogs — is silently denied (`NotAllowedError: Permission
    // denied`). The voice pipeline then never acquires a track, no
    // PeerConnection is created, and the user stays alone in the channel
    // (`sfu_active_peers=0`). This Chromium flag forces WebView2 to skip the
    // prompt and grant media access — appropriate for a voice client where
    // the user already opted into the desktop install.
    //
    // Must be set BEFORE `tauri::Builder::default()` so WebView2 picks it up
    // when it spawns. Multi-flag friendly: prepends to any existing value.
    #[cfg(target_os = "windows")]
    {
        const MEDIA_FLAGS: &str =
            "--use-fake-ui-for-media-stream --autoplay-policy=no-user-gesture-required";
        let merged = match std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
            Ok(existing) if !existing.is_empty() => format!("{MEDIA_FLAGS} {existing}"),
            _ => MEDIA_FLAGS.to_string(),
        };
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", merged);
    }

    let _ = rustls::crypto::ring::default_provider().install_default();

    let tls_config = tls::build_rustls_config();
    let client = tls::build_http_client(&tls_config);
    let ws_connector = tokio_tungstenite::Connector::Rustls(Arc::clone(&tls_config));

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_websocket::Builder::new()
                .tls_connector(ws_connector)
                .build(),
        )
        .manage(client)
        .manage(LayoutState::default())
        .invoke_handler(tauri::generate_handler![
            tls::call_signaling,
            tls::http_fetch,
            get_dsp_token,
            identity::create_identity,
            identity::find_identity_by_pubkey,
            identity::update_identity_pseudo,
            identity::update_identity_avatar,
            identity::recover_identity,
            identity::sign_message,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::include_image!("icons/icon.png");
                let _ = window.set_icon(icon);

                // Auto-grant mic/camera permission requests issued by the
                // renderer. Replaces the brittle
                // `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` env trick which is
                // ignored once WebView2 has cached a user-data folder.
                webview_perms::install_media_auto_grant(&window);
            }

            let identity_cache = identity::init_cache(&handle);
            app.manage(identity_cache);

            // Load layout from disk, discard legacy pixel-based layouts
            let initial = load_layout_from_disk(&handle)
                .filter(|l| !is_legacy_pixel_layout(l))
                .unwrap_or_else(default_layout);
            if let Ok(mut windows) = handle.state::<LayoutState>().windows.lock() {
                *windows = initial;
                let batch = LayoutBatch {
                    windows: windows.values().cloned().collect(),
                };
                let _ = handle.emit("bento:layout:update", batch);
            }

            let h_move = handle.clone();
            app.listen_any("bento:layout:move", move |event| {
                if let Ok(p) = serde_json::from_str::<MovePayload>(event.payload()) {
                    let state = h_move.state::<LayoutState>();
                    let _ = handle_move(&state, p, &h_move);
                }
            });

            let h_resize = handle.clone();
            app.listen_any("bento:layout:resize", move |event| {
                if let Ok(p) = serde_json::from_str::<ResizePayload>(event.payload()) {
                    let state = h_resize.state::<LayoutState>();
                    let _ = handle_resize(&state, p, &h_resize);
                }
            });

            let h_swap = handle.clone();
            app.listen_any("bento:layout:swap", move |event| {
                if let Ok(p) = serde_json::from_str::<SwapPayload>(event.payload()) {
                    let state = h_swap.state::<LayoutState>();
                    let _ = handle_swap(&state, p, &h_swap);
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_token_deterministic() {
        assert_eq!(get_dsp_token(), get_dsp_token());
    }

    #[test]
    fn dsp_token_nonzero() {
        assert_ne!(get_dsp_token(), 0);
    }

    #[test]
    fn layout_batch_serialization() {
        let batch = LayoutBatch {
            windows: vec![layout::LayoutWindow {
                id: "a".into(),
                x: 0.0,
                y: 0.0,
                w: 0.5,
                h: 0.5,
                z: 1,
            }],
        };
        let json = serde_json::to_string(&batch).unwrap();
        assert!(json.contains("\"windows\""));
        assert!(json.contains("\"a\""));
    }
}
