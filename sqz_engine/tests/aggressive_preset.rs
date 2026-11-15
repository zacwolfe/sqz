use sqz_engine::confidence_router::CompressionMode;
use sqz_engine::preset::{aggressive_preset, Preset};
use sqz_engine::SqzEngine;

#[test]
fn aggressive_preset_tightens_thresholds_and_cap() {
    let base = Preset::default();
    let aggressive = aggressive_preset(&base);
    assert_eq!(aggressive.compression.entropy_threshold, Some(200));
    assert_eq!(aggressive.compression.benign_entropy_threshold, Some(500));
    assert_eq!(aggressive.compression.max_output_tokens, Some(4000));
    assert_eq!(aggressive.preset.name, "aggressive");
}

#[test]
fn aggressive_mode_runs_and_reports_verify() {
    let dir = tempfile::tempdir().unwrap();
    let engine =
        SqzEngine::with_preset_and_store(Preset::default(), &dir.path().join("store.db")).unwrap();
    let input = (0..200)
        .map(|i| {
            format!(
                "2026-11-15T00:00:{:02}Z ERROR db timeout attempt {} connection refused",
                i % 60,
                i
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let result = engine
        .compress_with_mode(&input, CompressionMode::Aggressive)
        .expect("aggressive mode should return a result");
    assert!(!result.data.is_empty());
    assert!(
        result.verify.is_some(),
        "aggressive mode must attach verification"
    );
    assert!(result.tokens_compressed <= result.tokens_original.max(1));
}
