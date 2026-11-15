//! Every failed verifier invariant is a fallback trigger.
//!
//! The old rule derived fallback from an averaged confidence against a 0.6
//! threshold, so one failure out of seven checks (0.857) still shipped the
//! output. These tests pin the harder rule: a single failed check is enough,
//! while confidence stays a diagnostic that still reports how much survived.

use sqz_engine::Verifier;

const ORIGINAL: &str = "\
ok /repo/src/alpha.rs
ok /repo/src/beta.rs
artifact written to /repo/target/debug/sqz
fetched 3 packages in 1200 ms
cache hit ratio 0.87
";

#[test]
fn a_single_failed_invariant_triggers_fallback() {
    // Same paths, same words, but the numeric detail is gone: only
    // `numeric_values` fails, which the old averaged rule accepted.
    let compressed = "\
ok /repo/src/alpha.rs
ok /repo/src/beta.rs
artifact written to /repo/target/debug/sqz
fetched packages in ms
cache hit ratio
";
    let result = Verifier::verify(ORIGINAL, &compressed);
    assert_eq!(
        result.checks_failed.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        vec!["numeric_values"],
        "fixture must isolate a single non-preservation failure: {:?}",
        result.checks_failed
    );
    assert!(
        result.confidence >= 0.6,
        "fixture must exceed the old threshold, got {}",
        result.confidence
    );
    assert!(
        result.fallback_triggered,
        "a failed invariant must trigger fallback regardless of the average"
    );
    assert!(!result.passed);
}

#[test]
fn a_faithful_reduction_does_not_trigger_fallback() {
    let compressed = ORIGINAL.replace("artifact written to", "artifact:");
    let result = Verifier::verify(ORIGINAL, &compressed);
    assert!(result.checks_failed.is_empty(), "{:?}", result.checks_failed);
    assert!(!result.fallback_triggered);
    assert_eq!(result.confidence, 1.0);
    assert!(result.passed);
}

#[test]
fn identity_never_triggers_fallback() {
    let result = Verifier::verify(ORIGINAL, ORIGINAL);
    assert!(!result.fallback_triggered);
    assert!(result.passed);
}

/// Folding 120 near-identical log lines into one representative line plus a
/// marker is a ~3% ratio and the right answer. Retention is reported and
/// penalises confidence, but it must not send the caller back to lossless.
#[test]
fn an_aggressive_fold_is_not_rejected_for_being_short() {
    let mut original: String = (0..120)
        .map(|i| format!("2026-10-05T12:{:02}:{:02}Z INFO  GET /healthz 200 1ms\n", i / 60, i % 60))
        .collect();
    original.push_str("2026-10-05T12:30:00Z DEBUG db config host=db.internal user=app\n");
    let folded = "\
2026-10-05T12:00:00Z INFO  GET /healthz 200 1ms\n[\u{d7}119 similar lines; only timestamps/durations vary]\n2026-10-05T12:30:00Z DEBUG db config host=db.internal user=app\n";

    let result = Verifier::verify(&original, folded);
    assert!(
        result.checks_failed.iter().any(|(k, _)| k == "min_retention"),
        "retention should still be reported: {:?}",
        result.checks_failed
    );
    assert!(
        result.confidence < 1.0,
        "retention should still cost confidence, got {}",
        result.confidence
    );
    assert!(
        !result.fallback_triggered,
        "a correct fold must not fall back: {:?}",
        result.checks_failed
    );
}

/// A formatter's contract is narrower than the pipeline's: it may drop noise
/// rows, but never a line that carries diagnostic signal.
#[test]
fn a_formatter_may_drop_noise_but_not_signal() {
    let original = "\
   Compiling ledger v0.3.1 (/work/ledger)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 7.42s

running 300 tests
test tests::case_000 ... ok
test tests::case_001 ... FAILED
---- tests::case_001 stdout ----
thread 'tests::case_001' panicked at src/ledger.rs:41:9:
assertion `left == right` failed

test result: FAILED. 299 passed; 1 failed; 0 ignored
";
    let faithful = "\
FAILURES (1):
test tests::case_001 ... FAILED
---- tests::case_001 stdout ----
thread 'tests::case_001' panicked at src/ledger.rs:41:9:
assertion `left == right` failed
test result: FAILED. 299 passed; 1 failed; 0 ignored
";
    assert!(
        Verifier::formatter_losses(original, faithful).is_empty(),
        "dropping build noise is allowed: {:?}",
        Verifier::formatter_losses(original, faithful)
    );

    let lossy = faithful.replace("assertion `left == right` failed\n", "");
    assert!(
        !Verifier::formatter_losses(original, &lossy).is_empty(),
        "a dropped assertion line must be reported"
    );

    let reindented = faithful.replace("assertion `left", "      assertion `left");
    assert!(
        Verifier::formatter_losses(original, &reindented).is_empty(),
        "re-indenting a trace is not a loss: {:?}",
        Verifier::formatter_losses(original, &reindented)
    );
}
