// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// Use of this source code is governed by the LICENSE file at the
// repository root. Change Date: 2031-04-07. Change License:
// GPL-3.0-or-later.
// SPDX-License-Identifier: BUSL-1.1

//! Signaling server daemon entry point. The actual modules live in the
//! `signaling_server` library crate (`src/lib.rs`) so they can be reused
//! by integration tests and criterion benchmarks.

use signaling_server::{auth, fraud, friends, metrics, nonce, sfu, store};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{Extension, Router, routing::get};
use axum_server::tls_rustls::RustlsConfig;
use rustls::crypto::aws_lc_rs;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use void_sfu::{Sfu, SfuConfig};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::ice::candidate::CandidateType;
use webrtc::ice::udp_network::{EphemeralUDP, UDPNetwork};
use webrtc::interceptor::registry::Registry as InterceptorRegistry;

use sfu::adapter::WsRoomObserver;
use sfu::handler::ws_handler;
use sfu::registry::ServerRegistry;
use sfu::state::AppState;
use sfu::subscriptions::Subscriptions;

#[tokio::main]
async fn main() {
    let is_dev = std::env::var("DEV_MODE").is_ok();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(if is_dev { 8080 } else { 3001 });

    if !is_dev {
        if let Err(e) = aws_lc_rs::default_provider().install_default() {
            eprintln!("Failed to install aws-lc-rs crypto provider: {:?}", e);
            std::process::exit(1);
        }
    }

    tracing_subscriber::fmt::init();
    metrics::init_uptime();

    let api = match build_webrtc_api() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to build WebRTC API: {:?}", e);
            std::process::exit(1);
        }
    };
    let sfu = Sfu::with_api(SfuConfig::default(), api);

    let auth_store = store::Store::load("auth_store.bin");
    store::spawn_flusher(auth_store.clone());

    let server_registry = ServerRegistry::load("servers.bin");
    let server_registry_for_auth = server_registry.clone();
    sfu::registry::spawn_flusher(server_registry.clone());

    let app_state = Arc::new(AppState {
        peers: RwLock::new(HashMap::new()),
        chat_history: RwLock::new(HashMap::new()),
        dm_history: RwLock::new(HashMap::new()),
        server_registry,
        sfu: sfu.clone(),
        auth_store: auth_store.clone(),
        subscriptions: Subscriptions::new(),
    });

    // Wire the SFU's room-event observer to broadcast peer-joined / peer-left
    // messages to remaining members.
    sfu.set_observer(Arc::new(WsRoomObserver::new(Arc::clone(&app_state))));

    metrics::spawn_stats_broadcaster(Arc::clone(&app_state));

    // Fraud detection subsystem
    let ban_store = fraud::store::BanStore::load("ban_store.bin");
    fraud::store::spawn_flusher(ban_store.clone());

    let fraud_detector = Arc::new(fraud::detector::FraudDetector::new());
    fraud::detector::spawn_cleanup(Arc::clone(&fraud_detector));

    // Periodic active-bans gauge refresh
    {
        let ban_ref = ban_store.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                metrics::ACTIVE_BANS.set(ban_ref.entries.len() as i64);
            }
        });
    }

    let fraud_state = fraud::FraudState {
        bans: ban_store,
        detector: fraud_detector,
    };

    let nonce_store = nonce::NonceStore::new();
    nonce::spawn_cleanup(nonce_store.clone());

    let app: Router<()> = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "Healthy" }))
        .route("/metrics", get(metrics::handler))
        .route("/api/auth/nonce", get(nonce::get_nonce))
        .with_state(Arc::clone(&app_state))
        .nest(
            "/api/servers",
            sfu::routes::router().with_state(Arc::clone(&app_state)),
        )
        .nest(
            "/api/auth",
            auth::router()
                .with_state(auth_store.clone())
                .layer(Extension(server_registry_for_auth)),
        )
        .nest(
            "/api/friends",
            friends::router().with_state(Arc::clone(&app_state)),
        )
        .layer(Extension(fraud_state))
        .layer(Extension(nonce_store))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = match format!("0.0.0.0:{}", port).parse() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Invalid bind address: {:?}", e);
                std::process::exit(1);
            }
        };

        // On utilise le même serveur HTTP simple pour DEV et PROD.
        // En PROD, Nginx (port 443) réceptionne le HTTPS et le "traduit" en HTTP
        // vers notre port 3001.
        println!(
            "🚀 SFU Server running on http://{} | Mode: {} | UDP: 10000-20000",
            addr,
            if is_dev { "DEVELOPMENT" } else { "PRODUCTION" }
        );

        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Bind failed: {:?}", e);
                std::process::exit(1);
            }
        };

        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            eprintln!("Server error: {:?}", e);
            std::process::exit(1);
        }
    }

/// Builds a webrtc-rs `API` with default codecs, default interceptors,
/// and an ephemeral UDP port range of 10000..=20000.
fn build_webrtc_api() -> Result<webrtc::api::API, webrtc::Error> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;

    let mut setting_engine = SettingEngine::default();
    let ephemeral_udp = EphemeralUDP::new(10000, 20000)?;
    setting_engine.set_udp_network(UDPNetwork::Ephemeral(ephemeral_udp));

    // Oracle Cloud (and most IaaS) assigns a public IP via 1:1 NAT.
    // Without this, host candidates advertise 10.x / 172.x (unreachable)
    // and the srflx candidate often fails STUN checks behind cloud NAT.
    if let Ok(ip) = std::env::var("PUBLIC_IP") {
        setting_engine.set_nat_1to1_ips(vec![ip], CandidateType::Host.into());
        println!("🚀 WebRTC NAT 1:1 configuré avec l'IP: {}", std::env::var("PUBLIC_IP").unwrap());
    } else {
        println!("PUBLIC_IP non définie, passage en mode local (LAN)")
    }
    let mut registry = InterceptorRegistry::default();
    registry = register_default_interceptors(registry, &mut m)?;

    Ok(APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting_engine)
        .build())
}
