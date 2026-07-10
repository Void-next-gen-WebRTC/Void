use crate::nonce::NonceStore;

// ---------------------------------------------------------------------------
// 1. Generate returns a valid UUID-v4 string
// ---------------------------------------------------------------------------

#[test]
fn generate_returns_uuid() {
    let store = NonceStore::new();
    let nonce = store.generate().expect("generate");
    assert_eq!(nonce.len(), 36, "UUID v4 has 36 chars");
    assert_eq!(nonce.chars().filter(|&c| c == '-').count(), 4);
}

// ---------------------------------------------------------------------------
// 2. Consume succeeds on first use
// ---------------------------------------------------------------------------

#[test]
fn consume_valid_nonce() {
    let store = NonceStore::new();
    let nonce = store.generate().expect("generate");
    store.consume(&nonce).expect("first consume must succeed");
}

// ---------------------------------------------------------------------------
// 3. Double-consume fails (single-use guarantee)
// ---------------------------------------------------------------------------

#[test]
fn double_consume_fails() {
    let store = NonceStore::new();
    let nonce = store.generate().expect("generate");
    store.consume(&nonce).expect("first");
    let err = store.consume(&nonce);
    assert!(err.is_err(), "second consume must fail");
}

// ---------------------------------------------------------------------------
// 4. Unknown nonce is rejected
// ---------------------------------------------------------------------------

#[test]
fn unknown_nonce_rejected() {
    let store = NonceStore::new();
    let err = store.consume("totally-fake-nonce");
    assert!(err.is_err());
}

// ---------------------------------------------------------------------------
// 5. Concurrent generation produces unique nonces
// ---------------------------------------------------------------------------

#[test]
fn concurrent_generation_uniqueness() {
    use std::sync::Arc;
    use std::thread;

    let store = Arc::new(NonceStore::new());
    let handles: Vec<_> = (0..500)
        .map(|_| {
            let s = Arc::clone(&store);
            thread::spawn(move || s.generate().expect("generate"))
        })
        .collect();

    let nonces: Vec<String> = handles
        .into_iter()
        .map(|h| h.join().expect("join"))
        .collect();
    let mut unique = nonces.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 500, "all 500 nonces must be unique");
}

// ---------------------------------------------------------------------------
// 6. Concurrent consume: exactly one thread wins per nonce
// ---------------------------------------------------------------------------

#[test]
fn concurrent_consume_exactly_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    let store = Arc::new(NonceStore::new());
    let nonce = store.generate().expect("generate");
    let success_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..100)
        .map(|_| {
            let s = Arc::clone(&store);
            let n = nonce.clone();
            let cnt = Arc::clone(&success_count);
            thread::spawn(move || {
                if s.consume(&n).is_ok() {
                    cnt.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("join");
    }
    assert_eq!(
        success_count.load(Ordering::Relaxed),
        1,
        "exactly one consumer wins"
    );
}

// ---------------------------------------------------------------------------
// 7. DoS guard: MAX_PENDING limit
// ---------------------------------------------------------------------------

#[test]
fn max_pending_limit() {
    let store = NonceStore::new();
    // Generate up to the limit (10_000)
    for _ in 0..10_000 {
        store.generate().expect("within limit");
    }
    let err = store.generate();
    assert!(err.is_err(), "should reject beyond MAX_PENDING");
}
