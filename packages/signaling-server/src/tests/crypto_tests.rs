use base64::{engine::general_purpose, Engine as _};
use ed25519_dalek::{Signer, SigningKey};

use crate::sfu::crypto::verify_signature;

/// Generates an Ed25519 keypair and returns (public_key_b64, signing_key).
fn gen_keypair() -> (String, SigningKey) {
    let sk = SigningKey::generate(&mut rand::rngs::OsRng);
    let pk_b64 = general_purpose::STANDARD.encode(sk.verifying_key().as_bytes());
    (pk_b64, sk)
}

// ---------------------------------------------------------------------------
// 1. Valid signature verifies correctly
// ---------------------------------------------------------------------------

#[test]
fn valid_signature_verifies() {
    let (pk_b64, sk) = gen_keypair();
    let message = b"create:MyServer:some-nonce";
    let sig = sk.sign(message);
    let sig_b64 = general_purpose::STANDARD.encode(sig.to_bytes());

    let result = verify_signature(&pk_b64, message, &sig_b64).expect("no error");
    assert!(result, "valid signature must verify");
}

// ---------------------------------------------------------------------------
// 2. Wrong message fails verification
// ---------------------------------------------------------------------------

#[test]
fn wrong_message_fails() {
    let (pk_b64, sk) = gen_keypair();
    let sig = sk.sign(b"original message");
    let sig_b64 = general_purpose::STANDARD.encode(sig.to_bytes());

    let result = verify_signature(&pk_b64, b"tampered message", &sig_b64).expect("no error");
    assert!(!result, "tampered message must fail");
}

// ---------------------------------------------------------------------------
// 3. Wrong key fails verification
// ---------------------------------------------------------------------------

#[test]
fn wrong_key_fails() {
    let (_pk_b64, sk) = gen_keypair();
    let (other_pk_b64, _) = gen_keypair();
    let message = b"test message";
    let sig = sk.sign(message);
    let sig_b64 = general_purpose::STANDARD.encode(sig.to_bytes());

    let result = verify_signature(&other_pk_b64, message, &sig_b64).expect("no error");
    assert!(!result, "wrong key must fail");
}

// ---------------------------------------------------------------------------
// 4. Invalid base64 public key returns Err
// ---------------------------------------------------------------------------

#[test]
fn invalid_b64_pubkey_returns_err() {
    let result = verify_signature("not-valid-base64!!!", b"msg", "AAAA");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// 5. Wrong-length public key returns Err
// ---------------------------------------------------------------------------

#[test]
fn wrong_length_pubkey_returns_err() {
    let short = general_purpose::STANDARD.encode([0u8; 16]);
    let sig = general_purpose::STANDARD.encode([0u8; 64]);
    let result = verify_signature(&short, b"msg", &sig);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// 6. Wrong-length signature returns Err
// ---------------------------------------------------------------------------

#[test]
fn wrong_length_signature_returns_err() {
    let (pk_b64, _) = gen_keypair();
    let short_sig = general_purpose::STANDARD.encode([0u8; 32]);
    let result = verify_signature(&pk_b64, b"msg", &short_sig);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// 7. Deterministic: same keypair + message → same result
// ---------------------------------------------------------------------------

#[test]
fn deterministic_verification() {
    let (pk_b64, sk) = gen_keypair();
    let message = b"deterministic test";
    let sig = sk.sign(message);
    let sig_b64 = general_purpose::STANDARD.encode(sig.to_bytes());

    for _ in 0..100 {
        let result = verify_signature(&pk_b64, message, &sig_b64).expect("no error");
        assert!(result);
    }
}
