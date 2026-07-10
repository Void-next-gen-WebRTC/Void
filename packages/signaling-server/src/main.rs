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
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{routing::get, Extension, Router};
use rustls::crypto::aws_lc_rs;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use void_sfu::{Sfu, SfuConfig};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::ice::candidate::CandidateType;
use webrtc::ice::udp_mux::{UDPMuxDefault, UDPMuxParams};
use webrtc::ice::udp_network::UDPNetwork;
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

    let api = match build_webrtc_api().await {
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
        "🚀 SFU Server running on http://{} | Mode: {} | ICE UDP: port {}",
        addr,
        if is_dev { "DEVELOPMENT" } else { "PRODUCTION" },
        std::env::var("ICE_UDP_PORT").unwrap_or_else(|_| "10000".into()),
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
/// and a single UDP mux bound to `0.0.0.0:ICE_UDP_PORT` (default: 10000).
///
/// Using a [`UDPMuxDefault`] instead of [`EphemeralUDP`] is critical on
/// cloud hosts (Oracle Cloud, AWS, etc.) where the Docker daemon creates
/// bridge interfaces (`docker0`, `172.17.0.0/16`) alongside the real NIC.
/// `EphemeralUDP` allocates sockets on *every* interface; with `nat_1to1_ips`
/// those Docker-bound sockets are advertised as reachable host candidates but
/// are unreachable via the cloud's 1:1 NAT, causing ICE to fail for peers
/// whose STUN reflexive path happens to be paired against a Docker port.
///
/// `UDPMuxDefault` binds a single socket to `0.0.0.0`, demultiplexing
/// simultaneous peer connections via STUN username fragments. Every packet
/// arrives and leaves through the real NIC, making Oracle NAT work correctly
/// for all candidates.
async fn build_webrtc_api() -> Result<webrtc::api::API, Box<dyn std::error::Error>> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;

    let mut setting_engine = SettingEngine::default();

    // Bind a single UDP socket to 0.0.0.0. This prevents ICE sockets from
    // being allocated on Docker bridge IPs (172.17.x.x) which are unreachable
    // via Oracle Cloud's 1:1 NAT. The UDPMuxDefault demultiplexes multiple
    // simultaneous peer connections by STUN ufrag on this one socket.
    let udp_port: u16 = std::env::var("ICE_UDP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(10000);
    let listen_addr = format!("0.0.0.0:{}", udp_port);
    let udp_socket = tokio::net::UdpSocket::bind(&listen_addr)
        .await
        .map_err(|e| format!("Failed to bind ICE UDP socket on {listen_addr}: {e}"))?;
    // UDPMuxDefault::new already returns Arc<Self>; no extra Arc::new wrapper needed.
    let udp_mux = UDPMuxDefault::new(UDPMuxParams::new(udp_socket));
    setting_engine.set_udp_network(UDPNetwork::Muxed(udp_mux));
    println!("🔊 ICE UDP mux listening on {}", listen_addr);

    // Interface filter: kept for defence-in-depth on candidate advertisement.
    // With UDPMux the sockets are already on 0.0.0.0, so this mainly guards
    // against advertising Docker IPs as host candidates in non-NAT-1:1 mode.
    let deny_prefixes = parse_csv_env(
        "ICE_INTERFACE_DENY",
        "lo,docker,br-,veth,virbr,vmnet,cni,flannel",
    );
    let allow_prefixes = parse_csv_env("ICE_INTERFACE_ALLOW", "en,eth,ens,eno,enp,wlan,wl");
    setting_engine.set_interface_filter(Box::new(move |iface: &str| {
        interface_allowed(iface, &allow_prefixes, &deny_prefixes)
    }));

    // Oracle Cloud (and most IaaS) assigns a public IP via 1:1 NAT.
    // Without this, host candidates advertise 10.x (unreachable) and the
    // srflx candidate often fails STUN checks behind cloud NAT.
    if let Ok(ip) = std::env::var("PUBLIC_IP") {
        let trimmed = ip.trim();
        if trimmed.parse::<IpAddr>().is_ok() {
            setting_engine.set_nat_1to1_ips(vec![trimmed.to_string()], CandidateType::Host.into());
            println!("WebRTC NAT 1:1 configured with PUBLIC_IP={}", trimmed);
        } else {
            eprintln!("Ignoring invalid PUBLIC_IP value: {}", trimmed);
        }
    } else {
        println!("PUBLIC_IP is not set, running in local/LAN ICE mode");
    }

    let mut registry = InterceptorRegistry::default();
    registry = register_default_interceptors(registry, &mut m)?;

    Ok(APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting_engine)
        .build())
}

fn parse_csv_env(var_name: &str, default_value: &str) -> Vec<String> {
    std::env::var(var_name)
        .unwrap_or_else(|_| default_value.to_string())
        .split(',')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

fn interface_allowed(name: &str, allow_prefixes: &[String], deny_prefixes: &[String]) -> bool {
    let lower_name = name.to_ascii_lowercase();
    if deny_prefixes
        .iter()
        .any(|prefix| lower_name.starts_with(prefix))
    {
        return false;
    }

    allow_prefixes.is_empty()
        || allow_prefixes
            .iter()
            .any(|prefix| lower_name.starts_with(prefix))
}
