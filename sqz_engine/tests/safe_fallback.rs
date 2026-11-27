//! Fix: the "safe" fallback must not return a lossy result its own verifier
//! rejected. When verification fails, `compress_safe` now re-runs the lossless
//! path so the caller sees the bytes it passed in.
//!
//! These tests feed payloads the router classifies as high-risk (stack traces,
//! secrets, migrations) through `CompressionMode::Safe` and assert the
//! post-condition: the output is either byte-identical to the input or it is a
//! compression whose verifier actually passed.

use sqz_engine::{CompressionMode, SqzEngine};

fn engine() -> (SqzEngine, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("sessions.db");
    let engine =
        SqzEngine::with_preset_and_store(sqz_engine::Preset::default(), &store).expect("engine init");
    (engine, dir)
}

const STACK_TRACE: &str = "\
thread 'main' panicked at src/db.rs:42:9:
connection refused
stack backtrace:
   0: rust_begin_unwind
   1: core::panicking::panic_fmt
   2: sqz_engine::db::connect
   3: sqz_engine::main
note: run with `RUST_BACKTRACE=1` for a backtrace
";

const SECRET: &str = "\
DATABASE_URL=postgres://app:hunter2@db.internal:5432/prod
API_KEY=sk-live-9f8e7d6c5b4a3210
password=hunter2
token=ghp_abcdefghijklmnopqrstuvwxyz0123456789
";

const MIGRATION: &str = "\
BEGIN;
ALTER TABLE users ADD COLUMN email TEXT NOT NULL;
UPDATE users SET email = 'unknown@example.com' WHERE email IS NULL;
CREATE INDEX idx_users_email ON users (email);
COMMIT;
";

#[test]
fn safe_mode_never_returns_an_unverified_reduction() {
    let (engine, _dir) = engine();
    for (label, payload) in [
        ("stack trace", STACK_TRACE),
        ("secret", SECRET),
        ("migration", MIGRATION),
    ] {
        let result = engine
            .compress_with_mode(payload, CompressionMode::Safe)
            .unwrap_or_else(|e| panic!("{label}: compress_with_mode failed: {e}"));
        if result.data == payload {
            continue;
        }
        let verify = result
            .verify
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: compressed but no verifier result attached"));
        assert!(
            verify.passed,
            "{label}: returned output whose own verifier failed: {:?}",
            verify.checks_failed
        );
        assert!(
            !verify.fallback_triggered,
            "{label}: returned output the verifier wanted to fall back on"
        );
    }
}

#[test]
fn safe_mode_keeps_risky_payloads_intact() {
    let (engine, _dir) = engine();
    for (label, payload) in [
        ("stack trace", STACK_TRACE),
        ("secret", SECRET),
        ("migration", MIGRATION),
    ] {
        let result = engine
            .compress_with_mode(payload, CompressionMode::Safe)
            .unwrap_or_else(|e| panic!("{label}: compress_with_mode failed: {e}"));
        assert_eq!(
            result.data, payload,
            "{label}: safe mode altered a high-risk payload"
        );
        assert_eq!(result.provenance.label.as_deref(), Some("safe-fallback"));
    }
}
