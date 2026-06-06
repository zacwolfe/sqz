//! Ruby ecosystem formatters: rspec, rubocop, rake/minitest, bundle.
//!
//! Ported from rtk's per-runner filters, adapted to sqz's post-hoc model.
//! rtk wraps execution and injects `--format json`; sqz only sees whatever
//! output already happened. So each formatter tries JSON first (in case the
//! user passed `--format json` themselves) and falls back to a text parser.

use super::truncate::CAP_WARNINGS;
use serde::Deserialize;

// rspec failures carry full backtraces — show fewer than a generic warning list.
const MAX_RSPEC_FAILURES: usize = 5;
const MAX_RAKE_FAILURES: usize = CAP_WARNINGS;

// ── Shared helpers ───────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut)
}

/// Backtrace line from gems/rspec/ruby internals — not user code.
fn is_gem_backtrace(line: &str) -> bool {
    line.contains("/gems/")
        || line.contains("lib/rspec")
        || line.contains("lib/ruby/")
        || line.contains("vendor/bundle")
}

// ════════════════════════════════════════════════════════════════════════════
// RSpec
// ════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct RspecOutput {
    examples: Vec<RspecExample>,
    summary: RspecSummary,
}

#[derive(Deserialize)]
struct RspecExample {
    full_description: String,
    status: String,
    file_path: String,
    line_number: u32,
    exception: Option<RspecException>,
}

#[derive(Deserialize)]
struct RspecException {
    class: String,
    message: String,
    #[serde(default)]
    backtrace: Vec<String>,
}

#[derive(Deserialize)]
struct RspecSummary {
    duration: f64,
    example_count: usize,
    failure_count: usize,
    pending_count: usize,
    #[serde(default)]
    errors_outside_of_examples_count: usize,
}

pub fn format_rspec(output: &str) -> String {
    if output.trim().is_empty() {
        return "RSpec: No output".to_string();
    }

    // Happy path: user passed --format json.
    if let Ok(rspec) = serde_json::from_str::<RspecOutput>(output) {
        return build_rspec_summary(&rspec);
    }

    let stripped = strip_rspec_noise(output);
    if let Ok(rspec) = serde_json::from_str::<RspecOutput>(&stripped) {
        return build_rspec_summary(&rspec);
    }

    filter_rspec_text(&stripped)
}

/// Drop Spring preloader, SimpleCov coverage blocks, DEPRECATION warnings, the
/// "Finished in" timing line, and Capybara screenshot noise (keep path only).
fn strip_rspec_noise(output: &str) -> String {
    let mut result = Vec::new();
    let mut in_simplecov_block = false;

    for line in output.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();

        if lower.contains("running via spring preloader") {
            continue;
        }
        if trimmed.starts_with("DEPRECATION WARNING:") {
            continue;
        }
        if trimmed.starts_with("Finished in ") {
            continue;
        }

        let is_simplecov = lower.contains("coverage report")
            || lower.contains("simplecov")
            || lower.contains("coverage/")
            || lower.contains(".simplecov")
            || (lower.contains("all files") && lower.contains("lines"));
        if is_simplecov {
            in_simplecov_block = true;
            continue;
        }
        if in_simplecov_block {
            if trimmed.is_empty() {
                in_simplecov_block = false;
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("saved screenshot to ") {
            result.push(format!("[screenshot: {}]", rest.trim()));
            continue;
        }
        if let Some(idx) = trimmed.find("saved screenshot to ") {
            let path = &trimmed[idx + "saved screenshot to ".len()..];
            result.push(format!("[screenshot: {}]", path.trim()));
            continue;
        }

        result.push(line.to_string());
    }

    result.join("\n")
}

fn build_rspec_summary(rspec: &RspecOutput) -> String {
    let s = &rspec.summary;

    if s.example_count == 0 && s.errors_outside_of_examples_count == 0 {
        return "RSpec: No examples found".to_string();
    }
    if s.example_count == 0 && s.errors_outside_of_examples_count > 0 {
        return format!(
            "RSpec: {} errors outside of examples ({:.2}s)",
            s.errors_outside_of_examples_count, s.duration
        );
    }

    if s.failure_count == 0 && s.errors_outside_of_examples_count == 0 {
        let passed = s.example_count.saturating_sub(s.pending_count);
        let mut result = format!("✓ RSpec: {} passed", passed);
        if s.pending_count > 0 {
            result.push_str(&format!(", {} pending", s.pending_count));
        }
        result.push_str(&format!(" ({:.2}s)", s.duration));
        return result;
    }

    let passed = s
        .example_count
        .saturating_sub(s.failure_count + s.pending_count);
    let mut result = format!("RSpec: {} passed, {} failed", passed, s.failure_count);
    if s.pending_count > 0 {
        result.push_str(&format!(", {} pending", s.pending_count));
    }
    result.push_str(&format!(" ({:.2}s)\n", s.duration));

    let failures: Vec<&RspecExample> = rspec
        .examples
        .iter()
        .filter(|e| e.status == "failed")
        .collect();

    if failures.is_empty() {
        return result.trim().to_string();
    }

    result.push_str("\nFailures:\n");

    for (i, example) in failures.iter().take(MAX_RSPEC_FAILURES).enumerate() {
        result.push_str(&format!(
            "{}. ✗ {}\n   {}:{}\n",
            i + 1,
            example.full_description,
            example.file_path,
            example.line_number
        ));

        if let Some(exc) = &example.exception {
            let short_class = exc.class.rsplit("::").next().unwrap_or(&exc.class);
            let first_msg = exc.message.lines().next().unwrap_or("");
            result.push_str(&format!("   {}: {}\n", short_class, truncate(first_msg, 120)));

            for bt in &exc.backtrace {
                if !bt.contains("/gems/") && !bt.contains("lib/rspec") {
                    result.push_str(&format!("   {}\n", truncate(bt, 120)));
                    break;
                }
            }
        }

        if i < failures.len().min(MAX_RSPEC_FAILURES) - 1 {
            result.push('\n');
        }
    }

    if failures.len() > MAX_RSPEC_FAILURES {
        result.push_str(&format!(
            "\n... +{} more failures\n",
            failures.len() - MAX_RSPEC_FAILURES
        ));
    }

    result.trim().to_string()
}

/// Is this the rspec summary line, e.g. "9 examples, 2 failures"?
fn is_rspec_summary_line(line: &str) -> bool {
    line.contains("example") && (line.contains("failure") || line.contains("pending"))
}

/// State-machine text parser for documentation/progress format output.
fn filter_rspec_text(output: &str) -> String {
    #[derive(PartialEq)]
    enum State {
        Header,
        Failures,
        FailedExamples,
        Summary,
    }

    let mut state = State::Header;
    let mut failures: Vec<String> = Vec::new();
    let mut current_failure = String::new();
    let mut summary_line = String::new();

    for line in output.lines() {
        let trimmed = line.trim();

        match state {
            State::Header => {
                if trimmed == "Failures:" {
                    state = State::Failures;
                } else if trimmed == "Failed examples:" {
                    state = State::FailedExamples;
                } else if is_rspec_summary_line(trimmed) {
                    summary_line = trimmed.to_string();
                    state = State::Summary;
                }
            }
            State::Failures => {
                if is_numbered_failure(trimmed) {
                    if !current_failure.trim().is_empty() {
                        failures.push(compact_failure_block(&current_failure));
                    }
                    current_failure = trimmed.to_string();
                    current_failure.push('\n');
                } else if trimmed == "Failed examples:" {
                    if !current_failure.trim().is_empty() {
                        failures.push(compact_failure_block(&current_failure));
                    }
                    current_failure.clear();
                    state = State::FailedExamples;
                } else if is_rspec_summary_line(trimmed) {
                    if !current_failure.trim().is_empty() {
                        failures.push(compact_failure_block(&current_failure));
                    }
                    current_failure.clear();
                    summary_line = trimmed.to_string();
                    state = State::Summary;
                } else if !trimmed.is_empty() {
                    if is_gem_backtrace(trimmed) {
                        continue;
                    }
                    current_failure.push_str(trimmed);
                    current_failure.push('\n');
                }
            }
            State::FailedExamples => {
                if is_rspec_summary_line(trimmed) {
                    summary_line = trimmed.to_string();
                    state = State::Summary;
                }
            }
            State::Summary => break,
        }
    }

    if !current_failure.trim().is_empty() && state == State::Failures {
        failures.push(compact_failure_block(&current_failure));
    }

    if !summary_line.is_empty() {
        if failures.is_empty() {
            return format!("RSpec: {}", summary_line);
        }
        let mut result = format!("RSpec: {}\n", summary_line);
        for (i, failure) in failures.iter().take(MAX_RSPEC_FAILURES).enumerate() {
            result.push_str(&format!("{}. ✗ {}\n", i + 1, failure));
            if i < failures.len().min(MAX_RSPEC_FAILURES) - 1 {
                result.push('\n');
            }
        }
        if failures.len() > MAX_RSPEC_FAILURES {
            result.push_str(&format!(
                "\n... +{} more failures\n",
                failures.len() - MAX_RSPEC_FAILURES
            ));
        }
        return result.trim().to_string();
    }

    // No summary found anywhere — scan from the end.
    for line in output.lines().rev() {
        let t = line.trim();
        if is_rspec_summary_line(t) {
            return format!("RSpec: {}", t);
        }
    }

    // Last resort: last 5 non-empty lines.
    let tail: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = tail.len().saturating_sub(5);
    tail[start..].join("\n")
}

/// "1) User#full_name..." — leading digits then ')'.
fn is_numbered_failure(line: &str) -> bool {
    let trimmed = line.trim();
    if let Some(pos) = trimmed.find(')') {
        let prefix = &trimmed[..pos];
        !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

/// Compact a failure block: keep the message, the spec file:line, drop gem
/// backtrace.
fn compact_failure_block(block: &str) -> String {
    let mut spec_file = String::new();
    let mut kept_lines: Vec<String> = Vec::new();

    for line in block.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("# ./spec/") || t.starts_with("# ./test/") {
            spec_file = t.trim_start_matches("# ").to_string();
        } else if t.starts_with('#') && (t.contains("/gems/") || t.contains("lib/rspec")) {
            continue;
        } else {
            kept_lines.push(t.to_string());
        }
    }

    let mut result = kept_lines.join("\n   ");
    if !spec_file.is_empty() {
        result.push_str(&format!("\n   {}", spec_file));
    }
    result
}

// ════════════════════════════════════════════════════════════════════════════
// RuboCop
// ════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct RubocopOutput {
    files: Vec<RubocopFile>,
    summary: RubocopSummary,
}

#[derive(Deserialize)]
struct RubocopFile {
    path: String,
    offenses: Vec<RubocopOffense>,
}

#[derive(Deserialize)]
struct RubocopOffense {
    cop_name: String,
    severity: String,
    message: String,
    #[serde(default)]
    correctable: bool,
    location: RubocopLocation,
}

#[derive(Deserialize)]
struct RubocopLocation {
    start_line: usize,
}

#[derive(Deserialize)]
struct RubocopSummary {
    offense_count: usize,
    inspected_file_count: usize,
    #[serde(default)]
    correctable_offense_count: usize,
}

pub fn format_rubocop(output: &str) -> String {
    if output.trim().is_empty() {
        return "RuboCop: No output".to_string();
    }
    if let Ok(rubocop) = serde_json::from_str::<RubocopOutput>(output) {
        return filter_rubocop_json(&rubocop);
    }
    filter_rubocop_text(output)
}

/// Rank severity for ordering: lower = more severe.
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "fatal" | "error" => 0,
        "warning" => 1,
        "convention" | "refactor" | "info" => 2,
        _ => 3,
    }
}

fn filter_rubocop_json(rubocop: &RubocopOutput) -> String {
    let s = &rubocop.summary;

    if s.offense_count == 0 {
        return format!("ok ✓ rubocop ({} files)", s.inspected_file_count);
    }

    let correctable_count = if s.correctable_offense_count > 0 {
        s.correctable_offense_count
    } else {
        rubocop
            .files
            .iter()
            .flat_map(|f| &f.offenses)
            .filter(|o| o.correctable)
            .count()
    };

    let mut result = format!(
        "rubocop: {} offenses ({} files)\n",
        s.offense_count, s.inspected_file_count
    );

    let mut files_with_offenses: Vec<&RubocopFile> = rubocop
        .files
        .iter()
        .filter(|f| !f.offenses.is_empty())
        .collect();

    files_with_offenses.sort_by(|a, b| {
        let a_worst = a
            .offenses
            .iter()
            .map(|o| severity_rank(&o.severity))
            .min()
            .unwrap_or(3);
        let b_worst = b
            .offenses
            .iter()
            .map(|o| severity_rank(&o.severity))
            .min()
            .unwrap_or(3);
        a_worst.cmp(&b_worst).then(a.path.cmp(&b.path))
    });

    let max_files = 10;
    let max_offenses_per_file = 5;

    for file in files_with_offenses.iter().take(max_files) {
        result.push_str(&format!("\n{}\n", compact_ruby_path(&file.path)));

        let mut sorted_offenses: Vec<&RubocopOffense> = file.offenses.iter().collect();
        sorted_offenses.sort_by(|a, b| {
            severity_rank(&a.severity)
                .cmp(&severity_rank(&b.severity))
                .then(a.location.start_line.cmp(&b.location.start_line))
        });

        for offense in sorted_offenses.iter().take(max_offenses_per_file) {
            let first_msg_line = offense.message.lines().next().unwrap_or("");
            result.push_str(&format!(
                "  :{} {} — {}\n",
                offense.location.start_line, offense.cop_name, first_msg_line
            ));
        }
        if sorted_offenses.len() > max_offenses_per_file {
            result.push_str(&format!(
                "  … +{} more\n",
                sorted_offenses.len() - max_offenses_per_file
            ));
        }
    }

    if files_with_offenses.len() > max_files {
        result.push_str(&format!(
            "\n… +{} more files\n",
            files_with_offenses.len() - max_files
        ));
    }

    if correctable_count > 0 {
        result.push_str(&format!(
            "\n({} correctable, run `rubocop -A`)",
            correctable_count
        ));
    }

    result.trim().to_string()
}

fn filter_rubocop_text(output: &str) -> String {
    // Ruby/Bundler load errors first.
    for line in output.lines() {
        let t = line.trim();
        if t.contains("cannot load such file")
            || t.contains("Bundler::GemNotFound")
            || t.contains("Gem::MissingSpecError")
            || t.starts_with("rubocop: command not found")
            || t.starts_with("rubocop: No such file")
        {
            let lines: Vec<&str> = output.trim().lines().take(20).collect();
            let total = output.trim().lines().count();
            if total > 20 {
                return format!(
                    "RuboCop error:\n{}\n... ({} more lines)",
                    lines.join("\n"),
                    total - 20
                );
            }
            return format!("RuboCop error:\n{}", lines.join("\n"));
        }
    }

    for line in output.lines().rev() {
        let t = line.trim();
        if t.contains("inspected") && t.contains("autocorrected") {
            let files = extract_leading_number(t);
            let corrected = extract_autocorrect_count(t);
            if files > 0 && corrected > 0 {
                return format!(
                    "ok ✓ rubocop -A ({} files, {} autocorrected)",
                    files, corrected
                );
            }
            return format!("RuboCop: {}", t);
        }
        if t.contains("inspected") && (t.contains("offense") || t.contains("no offenses")) {
            if t.contains("no offenses") {
                let files = extract_leading_number(t);
                if files > 0 {
                    return format!("ok ✓ rubocop ({} files)", files);
                }
                return "ok ✓ rubocop (no offenses)".to_string();
            }
            return format!("RuboCop: {}", t);
        }
    }

    let tail: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = tail.len().saturating_sub(5);
    if tail.is_empty() {
        return "RuboCop: No output".to_string();
    }
    format!("RuboCop: {}", tail[start..].join("\n"))
}

fn extract_leading_number(s: &str) -> usize {
    s.split_whitespace()
        .next()
        .and_then(|w| w.parse().ok())
        .unwrap_or(0)
}

fn extract_autocorrect_count(s: &str) -> usize {
    for part in s.split(',').rev() {
        let t = part.trim();
        if t.contains("autocorrected") {
            return extract_leading_number(t);
        }
    }
    0
}

/// Compact a Ruby file path to the nearest Rails-convention directory.
fn compact_ruby_path(path: &str) -> String {
    let path = path.replace('\\', "/");

    for prefix in &[
        "app/models/",
        "app/controllers/",
        "app/views/",
        "app/helpers/",
        "app/services/",
        "app/jobs/",
        "app/mailers/",
        "lib/",
        "spec/",
        "test/",
        "config/",
    ] {
        if let Some(pos) = path.find(prefix) {
            return path[pos..].to_string();
        }
    }

    if let Some(pos) = path.rfind("/app/") {
        return path[pos + 1..].to_string();
    }
    if let Some(pos) = path.rfind('/') {
        return path[pos + 1..].to_string();
    }
    path
}

// ════════════════════════════════════════════════════════════════════════════
// rake test / rails test — Minitest output
// ════════════════════════════════════════════════════════════════════════════

pub fn format_rake(subcmd: Option<&str>, output: &str) -> Option<String> {
    // Only the test task produces parseable Minitest output. Other rake tasks
    // (db:migrate, assets:precompile, …) fall through to generic compression.
    match subcmd {
        Some("test") => Some(filter_minitest_output(output)),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
enum MinitestState {
    Header,
    Running,
    Failures,
}

fn filter_minitest_output(output: &str) -> String {
    let mut state = MinitestState::Header;
    let mut failures: Vec<String> = Vec::new();
    let mut current_failure: Vec<String> = Vec::new();
    let mut summary_line = String::new();

    for line in output.lines() {
        let trimmed = line.trim();

        // Summary line, e.g. "8 runs, 9 assertions, 1 failures, 0 errors, 0 skips".
        if (trimmed.contains(" runs,") || trimmed.contains(" tests,"))
            && trimmed.contains(" assertions,")
        {
            summary_line = trimmed.to_string();
            continue;
        }

        if trimmed == "# Running:" || trimmed.starts_with("Started with run options") {
            state = MinitestState::Running;
            continue;
        }
        if trimmed.starts_with("Finished in ") {
            state = MinitestState::Failures;
            continue;
        }

        match state {
            MinitestState::Header | MinitestState::Running => continue,
            MinitestState::Failures => {
                if is_minitest_failure_header(trimmed) {
                    if !current_failure.is_empty() {
                        failures.push(current_failure.join("\n"));
                        current_failure.clear();
                    }
                    current_failure.push(trimmed.to_string());
                } else if trimmed.is_empty() && !current_failure.is_empty() {
                    failures.push(current_failure.join("\n"));
                    current_failure.clear();
                } else if !trimmed.is_empty() {
                    current_failure.push(line.to_string());
                }
            }
        }
    }

    if !current_failure.is_empty() {
        failures.push(current_failure.join("\n"));
    }

    build_minitest_summary(&summary_line, &failures)
}

/// "1) Failure:" or "1) Error:".
fn is_minitest_failure_header(line: &str) -> bool {
    let line = line.trim();
    if let Some(pos) = line.find(')') {
        let prefix = &line[..pos];
        if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        let rest = line[pos + 1..].trim();
        rest == "Failure:" || rest == "Error:"
    } else {
        false
    }
}

fn build_minitest_summary(summary: &str, failures: &[String]) -> String {
    let (runs, fail_count, error_count, skips) = parse_minitest_summary(summary);

    if runs == 0 && summary.is_empty() {
        return "rake test: no tests ran".to_string();
    }

    if fail_count == 0 && error_count == 0 {
        let mut msg = format!("ok rake test: {} runs, 0 failures", runs);
        if skips > 0 {
            msg.push_str(&format!(", {} skips", skips));
        }
        return msg;
    }

    let mut result = format!(
        "rake test: {} runs, {} failures, {} errors",
        runs, fail_count, error_count
    );
    if skips > 0 {
        result.push_str(&format!(", {} skips", skips));
    }
    result.push('\n');

    if failures.is_empty() {
        return result.trim().to_string();
    }

    result.push('\n');

    for (i, failure) in failures.iter().take(MAX_RAKE_FAILURES).enumerate() {
        let lines: Vec<&str> = failure.lines().collect();
        if let Some(header) = lines.first() {
            result.push_str(&format!("{}. {}\n", i + 1, header.trim()));
        }
        for line in lines.iter().skip(1).take(4) {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                result.push_str(&format!("   {}\n", truncate(trimmed, 120)));
            }
        }
        if i < failures.len().min(MAX_RAKE_FAILURES) - 1 {
            result.push('\n');
        }
    }

    if failures.len() > MAX_RAKE_FAILURES {
        result.push_str(&format!(
            "\n... +{} more failures\n",
            failures.len() - MAX_RAKE_FAILURES
        ));
    }

    result.trim().to_string()
}

/// Returns (runs, failures, errors, skips).
fn parse_minitest_summary(summary: &str) -> (usize, usize, usize, usize) {
    let mut runs = 0;
    let mut failures = 0;
    let mut errors = 0;
    let mut skips = 0;

    for part in summary.split(',') {
        let words: Vec<&str> = part.trim().split_whitespace().collect();
        if words.len() >= 2 {
            if let Ok(n) = words[0].parse::<usize>() {
                match words[1] {
                    "runs" | "run" | "tests" | "test" => runs = n,
                    "failures" | "failure" => failures = n,
                    "errors" | "error" => errors = n,
                    "skips" | "skip" => skips = n,
                    _ => {}
                }
            }
        }
    }

    (runs, failures, errors, skips)
}

// ════════════════════════════════════════════════════════════════════════════
// bundle install / update
// ════════════════════════════════════════════════════════════════════════════

pub fn format_bundle(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd {
        Some("install") | Some("update") => Some(format_bundle_install(output)),
        _ => None,
    }
}

fn format_bundle_install(output: &str) -> String {
    if output.trim().is_empty() {
        return String::new();
    }

    if output.contains("Bundle updated!") {
        return "ok bundle: updated".to_string();
    }
    if output.contains("Bundle complete!") {
        return "ok bundle: complete".to_string();
    }

    // No completion line — likely an error. Keep installs + anything that looks
    // like an error, drop "Using"/resolving noise.
    let kept: Vec<&str> = output
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with("Using ")
                && !t.starts_with("Fetching gem metadata")
                && !t.starts_with("Resolving dependencies")
        })
        .collect();

    if kept.is_empty() {
        return "ok bundle".to_string();
    }
    let start = kept.len().saturating_sub(30);
    kept[start..].join("\n")
}

// ════════════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── RSpec JSON ────────────────────────────────────────────────────────────

    fn rspec_with_failures_json() -> &'static str {
        r#"{
          "examples": [
            {"full_description":"User is valid","status":"passed","file_path":"./spec/models/user_spec.rb","line_number":5,"exception":null},
            {"full_description":"User saves to database","status":"failed","file_path":"./spec/models/user_spec.rb","line_number":10,"exception":{
                "class":"RSpec::Expectations::ExpectationNotMetError",
                "message":"expected true but got false",
                "backtrace":[
                  "/usr/local/lib/ruby/gems/3.2.0/gems/rspec-expectations-3.12.0/lib/rspec/expectations/fail_with.rb:37:in `fail_with'",
                  "./spec/models/user_spec.rb:11:in `block (2 levels) in <top (required)>'"
                ]}}
          ],
          "summary": {"duration":0.123,"example_count":2,"failure_count":1,"pending_count":0,"errors_outside_of_examples_count":0}
        }"#
    }

    #[test]
    fn rspec_all_pass() {
        let json = r#"{"examples":[{"full_description":"a","status":"passed","file_path":"./spec/a_spec.rb","line_number":1,"exception":null}],"summary":{"duration":0.01,"example_count":1,"failure_count":0,"pending_count":0,"errors_outside_of_examples_count":0}}"#;
        let r = format_rspec(json);
        assert!(r.starts_with("✓ RSpec:"));
        assert!(r.contains("1 passed"));
    }

    #[test]
    fn rspec_failures_shortens_class_and_filters_gems() {
        let r = format_rspec(rspec_with_failures_json());
        assert!(r.contains("1 passed, 1 failed"));
        assert!(r.contains("✗ User saves to database"));
        assert!(r.contains("user_spec.rb:10"));
        assert!(r.contains("ExpectationNotMetError"));
        assert!(!r.contains("RSpec::Expectations::ExpectationNotMetError"));
        assert!(!r.contains("gems/rspec-expectations"));
        assert!(r.contains("user_spec.rb:11"));
    }

    #[test]
    fn rspec_empty() {
        assert_eq!(format_rspec(""), "RSpec: No output");
    }

    #[test]
    fn rspec_no_examples() {
        let json = r#"{"examples":[],"summary":{"duration":0.001,"example_count":0,"failure_count":0,"pending_count":0,"errors_outside_of_examples_count":0}}"#;
        assert_eq!(format_rspec(json), "RSpec: No examples found");
    }

    #[test]
    fn rspec_errors_outside_examples() {
        let json = r#"{"examples":[],"summary":{"duration":0.01,"example_count":0,"failure_count":0,"pending_count":0,"errors_outside_of_examples_count":1}}"#;
        let r = format_rspec(json);
        assert!(!r.contains("No examples found"));
        assert!(r.contains("errors outside"));
    }

    #[test]
    fn rspec_many_failures_caps_at_five() {
        let mut examples = Vec::new();
        for i in 0..6 {
            examples.push(format!(
                r#"{{"full_description":"t{}","status":"failed","file_path":"./spec/a_spec.rb","line_number":{},"exception":{{"class":"RuntimeError","message":"boom","backtrace":["./spec/a_spec.rb:{}:in `block'"]}}}}"#,
                i, i, i
            ));
        }
        let json = format!(
            r#"{{"examples":[{}],"summary":{{"duration":0.05,"example_count":6,"failure_count":6,"pending_count":0,"errors_outside_of_examples_count":0}}}}"#,
            examples.join(",")
        );
        let r = format_rspec(&json);
        assert!(r.contains("1. ✗"));
        assert!(r.contains("5. ✗"));
        assert!(!r.contains("6. ✗"));
        assert!(r.contains("+1 more"));
    }

    // ── RSpec text fallback ─────────────────────────────────────────────────

    #[test]
    fn rspec_text_fallback() {
        let text = "..F.\n\nFailures:\n\n  1) User is valid\n     Failure/Error: expect(user).to be_valid\n       expected true got false\n     # ./spec/models/user_spec.rb:5\n\n4 examples, 1 failure\n";
        let r = format_rspec(text);
        assert!(r.contains("RSpec:"));
        assert!(r.contains("4 examples, 1 failure"));
        assert!(r.contains("✗"));
        assert!(r.contains("spec/models/user_spec.rb:5"));
    }

    #[test]
    fn rspec_text_strips_spring_and_simplecov() {
        let text = "Running via Spring preloader in process 123\n....\n\nCoverage report generated for RSpec to /app/coverage.\n142 / 200 LOC (71.0%) covered.\n\n5 examples, 0 failures\n";
        let r = format_rspec(text);
        assert!(!r.contains("Spring"));
        assert!(!r.contains("Coverage"));
        assert!(r.contains("5 examples, 0 failures"));
    }

    #[test]
    fn rspec_text_screenshot_kept_as_path() {
        let text = "     saved screenshot to /tmp/capybara/failed.png\n3 examples, 1 failure\n";
        let stripped = strip_rspec_noise(text);
        assert!(stripped.contains("[screenshot:"));
        assert!(stripped.contains("failed.png"));
        assert!(!stripped.contains("saved screenshot to"));
    }

    #[test]
    fn rspec_invalid_json_no_panic() {
        let r = format_rspec("not json at all { broken");
        assert!(!r.is_empty());
    }

    // ── RuboCop ──────────────────────────────────────────────────────────────

    fn rubocop_with_offenses_json() -> &'static str {
        r#"{
          "files": [
            {"path":"app/models/user.rb","offenses":[
              {"severity":"convention","message":"Trailing whitespace detected.","cop_name":"Layout/TrailingWhitespace","correctable":true,"location":{"start_line":10}},
              {"severity":"warning","message":"Useless assignment to variable - `x`.","cop_name":"Lint/UselessAssignment","correctable":false,"location":{"start_line":25}}
            ]},
            {"path":"app/controllers/users_controller.rb","offenses":[
              {"severity":"error","message":"Syntax error.","cop_name":"Lint/Syntax","correctable":false,"location":{"start_line":30}}
            ]}
          ],
          "summary":{"offense_count":3,"target_file_count":2,"inspected_file_count":20,"correctable_offense_count":1}
        }"#
    }

    #[test]
    fn rubocop_no_offenses() {
        let json = r#"{"files":[],"summary":{"offense_count":0,"target_file_count":0,"inspected_file_count":15}}"#;
        assert_eq!(format_rubocop(json), "ok ✓ rubocop (15 files)");
    }

    #[test]
    fn rubocop_offenses_grouped_and_sorted() {
        let r = format_rubocop(rubocop_with_offenses_json());
        assert!(r.contains("3 offenses (20 files)"));
        // error-severity file sorts before convention/warning file
        let ctrl = r.find("users_controller.rb").unwrap();
        let model = r.find("app/models/user.rb").unwrap();
        assert!(ctrl < model);
        assert!(r.contains(":30 Lint/Syntax — Syntax error"));
        assert!(r.contains("1 correctable"));
    }

    #[test]
    fn rubocop_empty() {
        assert_eq!(format_rubocop(""), "RuboCop: No output");
    }

    #[test]
    fn rubocop_text_no_offenses() {
        let text = "Inspecting 10 files\n..........\n\n10 files inspected, no offenses detected";
        assert_eq!(format_rubocop(text), "ok ✓ rubocop (10 files)");
    }

    #[test]
    fn rubocop_text_autocorrect() {
        let text = "Inspecting 15 files\n...C..CC.......\n\n15 files inspected, 3 offenses detected, 3 offenses autocorrected";
        assert_eq!(
            format_rubocop(text),
            "ok ✓ rubocop -A (15 files, 3 autocorrected)"
        );
    }

    #[test]
    fn rubocop_text_bundler_error() {
        let text = "Bundler::GemNotFound: Could not find gem 'rubocop' in any sources.";
        let r = format_rubocop(text);
        assert!(r.starts_with("RuboCop error:"));
        assert!(r.contains("GemNotFound"));
    }

    #[test]
    fn rubocop_caps_files_at_ten() {
        let mut files = Vec::new();
        for i in 1..=12 {
            files.push(format!(
                r#"{{"path":"app/models/m_{}.rb","offenses":[{{"severity":"convention","message":"msg","cop_name":"Cop/X","correctable":false,"location":{{"start_line":1}}}}]}}"#,
                i
            ));
        }
        let json = format!(
            r#"{{"files":[{}],"summary":{{"offense_count":12,"target_file_count":12,"inspected_file_count":12}}}}"#,
            files.join(",")
        );
        let r = format_rubocop(&json);
        assert!(r.contains("… +2 more files"));
    }

    #[test]
    fn compact_ruby_path_works() {
        assert_eq!(
            compact_ruby_path("/home/user/project/app/models/user.rb"),
            "app/models/user.rb"
        );
        assert_eq!(
            compact_ruby_path("/project/spec/models/user_spec.rb"),
            "spec/models/user_spec.rb"
        );
        assert_eq!(compact_ruby_path("lib/tasks/deploy.rake"), "lib/tasks/deploy.rake");
    }

    #[test]
    fn severity_rank_ordering() {
        assert!(severity_rank("error") < severity_rank("warning"));
        assert!(severity_rank("warning") < severity_rank("convention"));
    }

    // ── rake / Minitest ──────────────────────────────────────────────────────

    #[test]
    fn rake_non_test_returns_none() {
        assert!(format_rake(Some("db:migrate"), "anything").is_none());
        assert!(format_rake(None, "anything").is_none());
    }

    #[test]
    fn minitest_all_pass() {
        let output = "Run options: --seed 12345\n\n# Running:\n\n........\n\nFinished in 0.123456s, 64.8 runs/s\n\n8 runs, 9 assertions, 0 failures, 0 errors, 0 skips";
        let r = format_rake(Some("test"), output).unwrap();
        assert!(r.contains("ok rake test"));
        assert!(r.contains("8 runs"));
        assert!(r.contains("0 failures"));
    }

    #[test]
    fn minitest_with_failures() {
        let output = "Run options: --seed 54321\n\n# Running:\n\n..F....\n\nFinished in 0.234567s, 29.8 runs/s\n\n  1) Failure:\nTestSomething#test_that_fails [/path/to/test.rb:15]:\nExpected: true\n  Actual: false\n\n7 runs, 7 assertions, 1 failures, 0 errors, 0 skips";
        let r = format_rake(Some("test"), output).unwrap();
        assert!(r.contains("1 failures"));
        assert!(r.contains("test_that_fails"));
        assert!(r.contains("Expected: true"));
    }

    #[test]
    fn minitest_empty() {
        let r = format_rake(Some("test"), "").unwrap();
        assert!(r.contains("no tests ran"));
    }

    #[test]
    fn minitest_reporters_format() {
        let output = "Started with run options --seed 37764\n\nProgress: |====|\n\nFinished in 5.79938s\n57 tests, 378 assertions, 0 failures, 0 errors, 0 skips";
        let r = format_rake(Some("test"), output).unwrap();
        assert!(r.contains("ok rake test"));
        assert!(r.contains("57 runs"));
    }

    #[test]
    fn minitest_skip() {
        let output = "# Running:\n\n..S..\n\nFinished in 0.1s, 50.0 runs/s\n\n5 runs, 4 assertions, 0 failures, 0 errors, 1 skips";
        let r = format_rake(Some("test"), output).unwrap();
        assert!(r.contains("ok rake test"));
        assert!(r.contains("1 skips"));
    }

    #[test]
    fn parse_minitest_summary_variants() {
        assert_eq!(
            parse_minitest_summary("8 runs, 9 assertions, 0 failures, 0 errors, 0 skips"),
            (8, 0, 0, 0)
        );
        assert_eq!(
            parse_minitest_summary("5 runs, 4 assertions, 1 failures, 1 errors, 2 skips"),
            (5, 1, 1, 2)
        );
        assert_eq!(
            parse_minitest_summary("57 tests, 378 assertions, 0 failures, 0 errors, 0 skips"),
            (57, 0, 0, 0)
        );
    }

    // ── bundle ───────────────────────────────────────────────────────────────

    #[test]
    fn bundle_complete_short_circuits() {
        let output = "Using bundler 2.5.6\nUsing rake 13.1.0\nBundle complete! 85 Gemfile dependencies, 200 gems now installed.";
        assert_eq!(
            format_bundle(Some("install"), output).unwrap(),
            "ok bundle: complete"
        );
    }

    #[test]
    fn bundle_updated() {
        let output = "Using rake 13.1.0\nInstalling rspec 3.14.0\nBundle updated!";
        assert_eq!(
            format_bundle(Some("update"), output).unwrap(),
            "ok bundle: updated"
        );
    }

    #[test]
    fn bundle_non_install_returns_none() {
        assert!(format_bundle(Some("exec"), "anything").is_none());
    }
}
