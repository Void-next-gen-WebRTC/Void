use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_int_counter, register_int_gauge, Encoder, Histogram, IntCounter,
    IntGauge, TextEncoder,
};

use crate::sfu::broadcast::serialize_message;
use crate::sfu::models::ServerMessage;
use crate::sfu::state::AppState;

// ---------------------------------------------------------------------------
// Existing metric definitions
// ---------------------------------------------------------------------------

pub static ACTIVE_PEERS: Lazy<IntGauge> =
    Lazy::new(|| register_int_gauge!("sfu_active_peers", "Number of connected peers").unwrap());

pub static ACTIVE_CHANNELS: Lazy<IntGauge> =
    Lazy::new(|| register_int_gauge!("sfu_active_channels", "Number of active channels").unwrap());

pub static BANDWIDTH_EGRESS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!("sfu_bandwidth_egress_bps", "Outgoing bandwidth (bits/s)").unwrap()
});

pub static BANDWIDTH_INGRESS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!("sfu_bandwidth_ingress_bps", "Incoming bandwidth (bits/s)").unwrap()
});

pub static PACKETS_PER_SEC: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "sfu_packets_per_second",
        "RTP packets per second",
        vec![100.0, 500.0, 1000.0, 5000.0, 10000.0]
    )
    .unwrap()
});

// ---------------------------------------------------------------------------
// New metrics — bounded channel drops
// ---------------------------------------------------------------------------

pub static WS_QUEUE_DROPPED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "sfu_ws_queue_dropped_total",
        "WebSocket messages dropped due to full bounded channel"
    )
    .unwrap()
});

pub static RTP_PACKETS_DROPPED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "sfu_rtp_packets_dropped_total",
        "RTP packets dropped due to full bounded channel"
    )
    .unwrap()
});

// ---------------------------------------------------------------------------
// New metrics — runtime state gauges
// ---------------------------------------------------------------------------

pub static PEER_CONNECTIONS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "sfu_peer_connections",
        "Number of active RTCPeerConnections"
    )
    .unwrap()
});

pub static FORWARDERS: Lazy<IntGauge> =
    Lazy::new(|| register_int_gauge!("sfu_forwarders", "Total active RTP forwarders").unwrap());

pub static DESTINATION_TRACKS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "sfu_destination_tracks",
        "Total destination tracks across all forwarders (O(N^2) indicator)"
    )
    .unwrap()
});

pub static MEMBERS_PER_CHANNEL: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "sfu_members_per_channel",
        "Distribution of members per active voice channel",
        vec![1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0]
    )
    .unwrap()
});

// ---------------------------------------------------------------------------
// New metrics — store / registry gauges
// ---------------------------------------------------------------------------

pub static REGISTERED_USERS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "sfu_registered_users",
        "Total registered users in auth store"
    )
    .unwrap()
});

pub static REGISTERED_SERVERS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!("sfu_registered_servers", "Total servers in registry").unwrap()
});

pub static ACTIVE_BANS: Lazy<IntGauge> =
    Lazy::new(|| register_int_gauge!("sfu_active_bans", "Currently active IP bans").unwrap());

pub static UPTIME_SECONDS: Lazy<IntGauge> =
    Lazy::new(|| register_int_gauge!("sfu_uptime_seconds", "Process uptime in seconds").unwrap());

/// Process start time — set once at startup.
static START_TIME: Lazy<Instant> = Lazy::new(Instant::now);

/// Must be called once at startup to initialise the start time.
pub fn init_uptime() {
    Lazy::force(&START_TIME);
}

// ---------------------------------------------------------------------------
// GET /metrics
// ---------------------------------------------------------------------------

/// Exposes Prometheus-compatible metrics.
pub async fn handler(state: axum::extract::State<Arc<AppState>>) -> String {
    let snapshot = state.sfu.metrics_snapshot().await;

    ACTIVE_PEERS.set(snapshot.peer_count as i64);
    ACTIVE_CHANNELS.set(snapshot.room_count as i64);
    PEER_CONNECTIONS.set(snapshot.peer_connections as i64);
    FORWARDERS.set(snapshot.total_forwarders as i64);
    DESTINATION_TRACKS.set(snapshot.total_dest_tracks as i64);

    let total_bandwidth: u64 = snapshot.peer_bandwidths.iter().map(|(_, b)| *b).sum();
    BANDWIDTH_EGRESS.set(total_bandwidth as i64);
    BANDWIDTH_INGRESS.set(total_bandwidth as i64);

    for count in &snapshot.members_per_room {
        MEMBERS_PER_CHANNEL.observe(*count as f64);
    }

    REGISTERED_USERS.set(state.auth_store.users.len() as i64);
    REGISTERED_SERVERS.set(state.server_registry.servers.len() as i64);
    UPTIME_SECONDS.set(START_TIME.elapsed().as_secs() as i64);

    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&prometheus::gather(), &mut buffer) {
        tracing::warn!("metrics encode failed: {:?}", e);
        return String::new();
    }
    String::from_utf8(buffer).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Background stats broadcaster (runs every 2 s)
// ---------------------------------------------------------------------------

/// Spawns a periodic task that pushes per-peer bandwidth stats over WS.
pub fn spawn_stats_broadcaster(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;

            let snapshot = state.sfu.metrics_snapshot().await;
            let total_packets: u64 = 0; // packets/s gauge moved to per-forwarder logging
            PACKETS_PER_SEC.observe(total_packets as f64);

            if snapshot.peer_bandwidths.is_empty() {
                continue;
            }

            let peers = state.peers.read().await;
            for (peer_id, bps) in snapshot.peer_bandwidths {
                let user_id = peer_id.to_string();
                if let Some(peer) = peers.get(&user_id) {
                    let msg = ServerMessage::Stats {
                        user_id: user_id.clone(),
                        bandwidth_bps: bps,
                    };
                    if let Some(payload) = serialize_message(&msg) {
                        if peer.tx.try_send(payload).is_err() {
                            WS_QUEUE_DROPPED.inc();
                        }
                    }
                }
            }
        }
    });
}
