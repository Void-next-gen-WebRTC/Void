use std::sync::LazyLock;

use crate::models::Claims;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

/// JWT secret cached at process start — avoids env-var lookup + heap allocation per call.
/// Panics in production if JWT_SECRET is not set or empty.
static JWT_SECRET: LazyLock<Vec<u8>> = LazyLock::new(|| match std::env::var("JWT_SECRET") {
    Ok(s) if !s.is_empty() => s.into_bytes(),
    _ => {
        if cfg!(test) || std::env::var("DEV_MODE").is_ok() {
            "dev-secret-do-not-use-in-prod".as_bytes().to_vec()
        } else {
            panic!("JWT_SECRET environment variable must be set in production")
        }
    }
});

/// Creates a signed JWT valid for 7 days.
pub fn create_token(user_id: &str) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 7 * 24 * 3600) as usize;

    let claims = Claims {
        sub: user_id.to_string(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(&JWT_SECRET),
    )
}

/// Decodes and validates a JWT, returning the embedded claims.
pub fn decode_token(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(&JWT_SECRET),
        &Validation::default(),
    )?;
    Ok(data.claims)
}
