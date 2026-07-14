use base64::{engine::general_purpose, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};

/// Verifies an Ed25519 signature against a public key and message.
/// Both `public_key_b64` and `signature_b64` are base64-encoded.
pub fn verify_signature(
    public_key_b64: &str,
    message: &[u8],
    signature_b64: &str,
) -> Result<bool, String> {
    let pk_bytes = general_purpose::STANDARD
        .decode(public_key_b64)
        .map_err(|e| format!("base64 decode public key: {e}"))?;

    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| "Public key must be 32 bytes".to_string())?;

    let verifying_key =
        VerifyingKey::from_bytes(&pk_array).map_err(|e| format!("invalid public key: {e}"))?;

    let sig_bytes = general_purpose::STANDARD
        .decode(signature_b64)
        .map_err(|e| format!("base64 decode signature: {e}"))?;

    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "Signature must be 64 bytes".to_string())?;

    let signature = Signature::from_bytes(&sig_array);

    Ok(verifying_key.verify_strict(message, &signature).is_ok())
}
