use axum::body::Bytes;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use prost::Message;
use serde::Serialize;

use crate::errors::ApiError;

// ---------------------------------------------------------------------------
// Header helpers
// ---------------------------------------------------------------------------

/// Checks if the client prefers protobuf responses.
pub fn accepts_proto(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/x-protobuf"))
}

/// Decodes a request body as protobuf or JSON depending on Content-Type.
pub fn decode_body<T: serde::de::DeserializeOwned + Message + Default>(
    headers: &HeaderMap,
    bytes: &Bytes,
) -> Result<T, ApiError> {
    let is_proto = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/x-protobuf"));

    if is_proto {
        T::decode(bytes.as_ref()).map_err(|e| ApiError::BadRequest(format!("protobuf decode: {e}")))
    } else {
        serde_json::from_slice(bytes).map_err(|e| ApiError::BadRequest(format!("json decode: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

/// Negotiated HTTP response (JSON or protobuf).
pub enum Negotiated {
    Json(Response),
    Proto(Vec<u8>),
}

impl IntoResponse for Negotiated {
    fn into_response(self) -> Response {
        match self {
            Negotiated::Json(r) => r,
            Negotiated::Proto(bytes) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-protobuf")],
                bytes,
            )
                .into_response(),
        }
    }
}

/// Produces a negotiated single-item response.
pub fn negotiate<T: Serialize + Message>(data: T, proto: bool) -> Negotiated {
    if proto {
        Negotiated::Proto(data.encode_to_vec())
    } else {
        Negotiated::Json(axum::Json(data).into_response())
    }
}

/// Produces a negotiated list response with a proto wrapper message.
pub fn negotiate_list<T, W>(
    items: Vec<T>,
    wrap: impl FnOnce(Vec<T>) -> W,
    proto: bool,
) -> Negotiated
where
    T: Serialize,
    W: Message,
{
    if proto {
        Negotiated::Proto(wrap(items).encode_to_vec())
    } else {
        Negotiated::Json(axum::Json(items).into_response())
    }
}
