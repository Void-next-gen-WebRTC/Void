use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue};
use prost::Message;

use crate::models::RegisterBody;
use crate::models::{StatusResponse, UserSummary, UserSummaryList};
use crate::negotiate::{accepts_proto, decode_body, negotiate, negotiate_list, Negotiated};

// ---------------------------------------------------------------------------
// 1. accepts_proto with application/x-protobuf header
// ---------------------------------------------------------------------------

#[test]
fn accepts_proto_with_proto_header() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/x-protobuf"),
    );
    assert!(accepts_proto(&headers));
}

// ---------------------------------------------------------------------------
// 2. accepts_proto with JSON accept header
// ---------------------------------------------------------------------------

#[test]
fn accepts_proto_with_json_header() {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    assert!(!accepts_proto(&headers));
}

// ---------------------------------------------------------------------------
// 3. accepts_proto with no accept header
// ---------------------------------------------------------------------------

#[test]
fn accepts_proto_with_no_header() {
    let headers = HeaderMap::new();
    assert!(!accepts_proto(&headers));
}

// ---------------------------------------------------------------------------
// 4. decode_body — JSON path
// ---------------------------------------------------------------------------

#[test]
fn decode_body_json() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let json = serde_json::json!({
        "username": "test",
        "displayName": "Test",
        "publicKey": "pk",
        "nonce": "n",
        "signature": "s"
    });
    let bytes = Bytes::from(serde_json::to_vec(&json).unwrap());
    let result: RegisterBody = decode_body(&headers, &bytes).expect("json decode");
    assert_eq!(result.username, "test");
    assert_eq!(result.public_key, "pk");
}

// ---------------------------------------------------------------------------
// 5. decode_body — protobuf path
// ---------------------------------------------------------------------------

#[test]
fn decode_body_protobuf() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-protobuf"),
    );
    let body = RegisterBody {
        username: "proto_user".into(),
        display_name: "Proto".into(),
        public_key: "pk123".into(),
        nonce: "nonce1".into(),
        signature: "sig1".into(),
    };
    let buf = body.encode_to_vec();
    let bytes = Bytes::from(buf);
    let decoded: RegisterBody = decode_body(&headers, &bytes).expect("proto decode");
    assert_eq!(decoded.username, "proto_user");
}

// ---------------------------------------------------------------------------
// 6. decode_body — invalid JSON returns Err
// ---------------------------------------------------------------------------

#[test]
fn decode_body_invalid_json() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let bytes = Bytes::from_static(b"not json");
    let result: Result<RegisterBody, _> = decode_body(&headers, &bytes);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// 7. decode_body — invalid protobuf returns Err
// ---------------------------------------------------------------------------

#[test]
fn decode_body_invalid_protobuf() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-protobuf"),
    );
    let bytes = Bytes::from_static(b"\xff\xff\xff");
    let result: Result<RegisterBody, _> = decode_body(&headers, &bytes);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// 8. negotiate — JSON path produces Negotiated::Json
// ---------------------------------------------------------------------------

#[test]
fn negotiate_json_response() {
    let data = StatusResponse {
        status: "ok".into(),
    };
    let result = negotiate(data, false);
    assert!(matches!(result, Negotiated::Json(_)));
}

// ---------------------------------------------------------------------------
// 9. negotiate — proto path produces Negotiated::Proto
// ---------------------------------------------------------------------------

#[test]
fn negotiate_proto_response() {
    let data = StatusResponse {
        status: "ok".into(),
    };
    let result = negotiate(data, true);
    match result {
        Negotiated::Proto(bytes) => {
            let decoded = StatusResponse::decode(bytes.as_slice()).unwrap();
            assert_eq!(decoded.status, "ok");
        }
        _ => panic!("expected Proto variant"),
    }
}

// ---------------------------------------------------------------------------
// 10. negotiate_list — JSON path
// ---------------------------------------------------------------------------

#[test]
fn negotiate_list_json() {
    let items = vec![UserSummary {
        id: "1".into(),
        username: "u".into(),
        display_name: "U".into(),
        avatar: None,
        public_key: None,
    }];
    let result = negotiate_list(items, |i| UserSummaryList { items: i }, false);
    assert!(matches!(result, Negotiated::Json(_)));
}

// ---------------------------------------------------------------------------
// 11. negotiate_list — proto path
// ---------------------------------------------------------------------------

#[test]
fn negotiate_list_proto() {
    let items = vec![UserSummary {
        id: "1".into(),
        username: "u".into(),
        display_name: "U".into(),
        avatar: None,
        public_key: None,
    }];
    let result = negotiate_list(items, |i| UserSummaryList { items: i }, true);
    match result {
        Negotiated::Proto(bytes) => {
            let decoded = UserSummaryList::decode(bytes.as_slice()).unwrap();
            assert_eq!(decoded.items.len(), 1);
        }
        _ => panic!("expected Proto variant"),
    }
}
