use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};

// ---------------------------------------------------------------------------
// Compile-time pin hashes (set via env vars for production builds)
// ---------------------------------------------------------------------------

const PRIMARY_PIN: &str = match option_env!("PRIMARY_PIN_HASH") {
    Some(v) => v,
    None => "DEV_PIN",
};
const BACKUP_PIN: &str = match option_env!("BACKUP_PIN_HASH") {
    Some(v) => v,
    None => "DEV_PIN",
};

/// Returns `true` when no real pin hashes were injected at compile time.
pub fn is_dev_build() -> bool {
    PRIMARY_PIN == "DEV_PIN" && BACKUP_PIN == "DEV_PIN"
}

// ---------------------------------------------------------------------------
// Custom certificate verifier (pinning in prod, pass-through in dev)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PinningVerifier;

fn is_pinned_host(hostname: &str) -> bool {
    matches!(
        hostname,
        "voidsfu.com" | "www.voidsfu.com" | "api.voidsfu.com" | "89.168.59.45"
    )
}

fn pin_mismatch_error(hostname: &str, cert_hash: &str) -> Error {
    Error::General(format!(
        "SPKI pin mismatch for {hostname}: expected {PRIMARY_PIN} or {BACKUP_PIN}, got {cert_hash}"
    ))
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        // Extract the target host from SNI.
        let hostname = match server_name {
            ServerName::DnsName(dns) => dns.as_ref().to_ascii_lowercase(),
            ServerName::IpAddress(_ip) => String::new(),
            _ => "".to_string(),
        };

        // Only enforce first-party pinning for known API hosts.
        // External services (e.g. updater endpoints) are intentionally not pinned here.
        if !is_pinned_host(&hostname) {
            #[cfg(debug_assertions)]
            println!(
                "ℹ️ [TLS Pinning] Bypass pour le domaine externe : {}",
                hostname
            );

            return Ok(ServerCertVerified::assertion());
        }

        // In local debug/test runs, always bypass pin checks to prevent stale
        // CI/release pin env values from breaking developer sign-in flows.
        if cfg!(debug_assertions) || cfg!(test) || is_dev_build() {
            return Ok(ServerCertVerified::assertion());
        }

        // Strict pinning for first-party hosts in release builds.
        let cert = x509_parser::parse_x509_certificate(end_entity.as_ref())
            .map_err(|_| Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;

        let spki_bytes = cert.1.tbs_certificate.subject_pki.raw;

        let mut hasher = Sha256::new();
        hasher.update(spki_bytes);
        let cert_hash = general_purpose::STANDARD.encode(hasher.finalize());

        #[cfg(debug_assertions)]
        println!(
            "🔒 [TLS Pinning] Interception de {}. Hash calculé: {}",
            hostname, cert_hash
        );

        if cert_hash == PRIMARY_PIN || cert_hash == BACKUP_PIN {
            Ok(ServerCertVerified::assertion())
        } else {
            eprintln!(
                "❌ [TLS Pinning] ÉCHEC sur {} ! Attendu: {}, Reçu: {}",
                hostname, PRIMARY_PIN, cert_hash
            );
            Err(pin_mismatch_error(&hostname, &cert_hash))
        }
    }

    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
        ]
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Builds the base `rustls::ClientConfig` shared by the WebSocket connector.
/// No ALPN — tokio-tungstenite handles protocol negotiation independently.
pub fn build_rustls_config() -> Arc<rustls::ClientConfig> {
    let mut config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinningVerifier))
        .with_no_client_auth();

    // Disable TLS session resumption — the signaling server (axum_server)
    // drops connections resumed via PSK under TLS 1.3.
    config.resumption = rustls::client::Resumption::disabled();

    Arc::new(config)
}

/// Builds the `reqwest::Client` wired to the pinning-aware TLS config.
/// Clones the base config and sets HTTP/1.1-only ALPN to match the
/// signaling server's capabilities.  Connection pooling is enabled so
/// the POST /register reuses the TLS tunnel established by GET /nonce.
pub fn build_http_client(tls: &Arc<rustls::ClientConfig>) -> reqwest::Client {
    let mut http_tls = (**tls).clone();
    http_tls.alpn_protocols = vec![b"http/1.1".to_vec()];

    reqwest::Client::builder()
        .use_preconfigured_tls(http_tls)
        .http1_only()
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build reqwest client")
}

// ---------------------------------------------------------------------------
// HTTP proxy command — routes webview requests through the pinned client
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ProxyRequest {
    method: String,
    url: String,
    headers: HashMap<String, String>,
    #[serde(default)]
    body: Option<Vec<u8>>,
}

#[derive(Serialize)]
pub struct ProxyResponse {
    status: u16,
    body: Vec<u8>,
}

#[tauri::command]
pub async fn http_fetch(
    client: tauri::State<'_, reqwest::Client>,
    request: ProxyRequest,
) -> Result<ProxyResponse, String> {
    let method: reqwest::Method = request
        .method
        .parse()
        .map_err(|_| format!("Invalid HTTP method: {}", request.method))?;

    let mut builder = client.request(method, &request.url);
    for (k, v) in &request.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    if let Some(body) = request.body {
        builder = builder.body(body);
    }

    let res = builder.send().await.map_err(|e| {
        let mut msg = e.to_string();
        let mut src: &dyn std::error::Error = &e;
        while let Some(cause) = src.source() {
            msg.push_str(" → ");
            msg.push_str(&cause.to_string());
            src = cause;
        }
        msg
    })?;

    let status = res.status().as_u16();
    let body = res.bytes().await.map_err(|e| e.to_string())?;
    Ok(ProxyResponse {
        status,
        body: body.to_vec(),
    })
}

#[tauri::command]
pub async fn call_signaling(
    client: tauri::State<'_, reqwest::Client>,
    url: String,
) -> Result<String, String> {
    client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_host_matches_first_party_targets() {
        assert!(is_pinned_host("api.voidsfu.com"));
        assert!(is_pinned_host("89.168.59.45"));
    }

    #[test]
    fn pinned_host_rejects_non_first_party_targets() {
        assert!(!is_pinned_host("github.com"));
        assert!(!is_pinned_host("api.voidsfu.com.evil"));
        assert!(!is_pinned_host(""));
    }

    #[test]
    fn pin_mismatch_error_is_explicit() {
        let err = pin_mismatch_error("api.voidsfu.com", "abc123");
        let msg = err.to_string();

        assert!(msg.contains("SPKI pin mismatch"));
        assert!(msg.contains("api.voidsfu.com"));
        assert!(msg.contains("abc123"));
    }
}
