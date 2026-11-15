//! Noise-run coalescing for terminal progress output.
//!
//! Builds, package installs, transfers, and long test runs redraw a single
//! row or repeat one transient status line. Those lines carry no durable
//! content, but every other stage keys on stable text: run-length encoding
//! needs byte-identical neighbours, `log_template` masks timestamps,
//! `collapse_indexed_rows` needs a varying index, and the entropy truncator
//! scores them as content it must keep. So progress output survives verbatim
//! and costs tokens on every command an agent runs.
//!
//! This stage handles the two shapes those stages miss:
//!
//! 1. **Frame redraws.** A progress bar is one logical line written many times
//!    with `\r`. The bytes contain every frame, but a terminal only ever shows
//!    the last one, so the last frame is what the reader would have seen. The
//!    frame count is declared in the marker.
//! 2. **Noise runs.** Consecutive lines that are each transient progress
//!    (bars, transfer status, spinner frames, "Downloading ..." lines) collapse
//!    to the first line plus a count of how many followed.
//!
//! Safety rules, all enforced before anything is dropped:
//!
//! - A line containing an error/warning/failure marker is never dropped, and a
//!   run containing one is never collapsed. Errors are the reason an agent read
//!   the output in the first place.
//! - Diff payload, JSON, and source code are refused wholesale.
//! - Every dropped line is declared: `[xN]` for identical lines, `[+N similar
//!   lines]` for a run that was not identical, `[xN frames]` for redraws.

use crate::error::Result;

/// Result of noise coalescing.
#[derive(Debug, Clone)]
pub struct NoiseRunResult {
    /// The coalesced text.
    pub text: String,
    /// Number of noisy runs replaced with a marker.
    pub runs_collapsed: usize,
    /// Number of whole lines the markers replaced.
    pub lines_removed: usize,
    /// Number of lines whose `\r` redraw frames were reduced to the last frame.
    pub frames_collapsed: usize,
    /// Tokens saved (estimated from bytes removed, as elsewhere in the engine).
    pub tokens_saved: u32,
}

/// A redraw below this many frames is ordinary carriage-return usage, not a
/// progress display.
const MIN_FRAMES: usize = 3;

/// Shortest input worth scanning: below this a "run" is just a few status
/// lines, and a marker would cost as much as the lines it replaces.
const MIN_INPUT_LINES: usize = 5;

/// Minimum fill characters inside a bracketed bar before it counts as a bar.
const MIN_BAR_FILL: usize = 4;

/// Minimum length of a dot/dash trail (`......`) to count as progress.
const MIN_DOT_TRAIL: usize = 6;

/// Words that mean "this line is not the result, it is the wait". Deliberately
/// excludes verbs that carry a durable noun (`Compiling foo v1.2.3` names a
/// crate, `Downloaded` is a receipt) and every verb that can introduce a
/// failure, since a failure line is never noise.
const PROGRESS_VERBS: [&str; 14] = [
    "downloading",
    "resolving",
    "fetching",
    "waiting",
    "preparing",
    "pulling",
    "extracting",
    "unpacking",
    "uploading",
    "installing",
    "receiving",
    "connecting",
    "scanning",
    "warming",
];

/// Tokens that mark a line as content an agent needs. A line containing any of
/// these is never treated as noise, whichever shape it otherwise matches.
const SIGNAL_WORDS: [&str; 12] = [
    "error",
    "warning",
    "warn:",
    "fail",
    "fatal",
    "panic",
    "traceback",
    "exception",
    "denied",
    "refused",
    "not found",
    "abort",
];

/// Coalesce terminal progress output.
///
/// `min_run` is the shortest run of consecutive noisy lines worth replacing
/// with one marker; anything shorter is left byte-exact, since a marker for
/// two lines usually does not shrink.
pub fn collapse_noise_runs(text: &str, min_run: usize) -> Result<NoiseRunResult> {
    let identity = || NoiseRunResult {
        text: text.to_string(),
        runs_collapsed: 0,
        lines_removed: 0,
        frames_collapsed: 0,
        tokens_saved: 0,
    };

    let min_run = min_run.max(2);
    if text.is_empty() {
        return Ok(identity());
    }

    // Structured payload and diffs are never progress output, and touching
    // them risks the same content other stages are relied on to preserve.
    let head = text.trim_start();
    if head.starts_with('{')
        || head.starts_with('[')
        || head.starts_with('<')
        || head.starts_with("#!/")
    {
        return Ok(identity());
    }
    if text.lines().any(|line| {
        line.starts_with("diff --git")
            || line.starts_with("@@")
            || line.starts_with("+++ ")
            || line.starts_with("--- ")
    }) {
        return Ok(identity());
    }

    // A redraw bar is one line carrying every frame, so the line count says
    // nothing about how much noise is present; only the run pass needs enough
    // lines to be worth a marker.
    let redraws = text.matches('\r').count();
    if redraws == 0 && text.lines().count() < MIN_INPUT_LINES {
        return Ok(identity());
    }

    let ends_with_newline = text.ends_with('\n');

    // Step 1: reduce `\r` redraws to the frame a terminal would have shown.
    let mut lines: Vec<String> = Vec::with_capacity(text.lines().count());
    let mut frames_collapsed = 0usize;
    for line in text.lines() {
        match last_frame(line) {
            Some(collapsed) => {
                lines.push(collapsed);
                frames_collapsed += 1;
            }
            None => lines.push(line.to_string()),
        }
    }

    // Step 2: collapse runs of consecutive transient lines.
    let mut output: Vec<String> = Vec::with_capacity(lines.len());
    let mut runs_collapsed = 0usize;
    let mut lines_removed = 0usize;
    let mut i = 0;
    while i < lines.len() {
        let mut end = i + 1;
        if is_noise_line(&lines[i]) {
            while end < lines.len() && is_noise_line(&lines[end]) {
                end += 1;
            }
        }
        let run_len = end - i;
        if run_len >= min_run {
            let identical = lines[i + 1..end].iter().all(|line| *line == lines[i]);
            if identical {
                output.push(format!("{} [x{run_len}]", lines[i]));
            } else {
                output.push(format!(
                    "{} [+{} similar lines]",
                    lines[i],
                    run_len - 1
                ));
            }
            runs_collapsed += 1;
            lines_removed += run_len - 1;
        } else {
            output.extend(lines[i..end].iter().cloned());
        }
        i = end;
    }

    if runs_collapsed == 0 && frames_collapsed == 0 {
        return Ok(identity());
    }

    let mut joined = output.join("\n");
    if ends_with_newline {
        joined.push('\n');
    }
    let removed_bytes = text.len().saturating_sub(joined.len());
    Ok(NoiseRunResult {
        tokens_saved: (removed_bytes / 4).min(u32::MAX as usize) as u32,
        text: joined,
        runs_collapsed,
        lines_removed,
        frames_collapsed,
    })
}

/// If `line` carries several `\r`-separated redraws, return the last non-empty
/// frame plus a count. A line with any error marker is returned untouched so a
/// failure printed mid-redraw is not lost.
fn last_frame(line: &str) -> Option<String> {
    if !line.contains('\r') {
        return None;
    }
    let frames: Vec<&str> = line
        .split('\r')
        .map(|frame| frame.trim_end())
        .filter(|frame| !frame.trim().is_empty())
        .collect();
    if frames.len() < MIN_FRAMES {
        return None;
    }
    if frames.iter().any(|frame| has_signal(frame)) {
        return None;
    }
    let last = frames.last()?;
    Some(format!("{last} [x{} frames]", frames.len()))
}

/// Whether a line is transient progress output rather than content.
fn is_noise_line(line: &str) -> bool {
    if has_signal(line) {
        return false;
    }
    has_bar(line)
        || has_transfer_status(line)
        || has_dot_trail(line)
        || starts_with_spinner(line)
        || starts_with_progress_verb(line)
}

/// True when the line carries an error/warning marker that must survive.
fn has_signal(line: &str) -> bool {
    let lower = line.to_lowercase();
    SIGNAL_WORDS.iter().any(|word| lower.contains(word))
}

/// A bracketed progress bar: `[====>   ]`, `[#####    ]`. An unbracketed fill
/// counts only when the same line carries a percentage, so `=====` header rules
/// and `| ++++----` diffstat rows are not mistaken for progress.
fn has_bar(line: &str) -> bool {
    let inner = match (line.find('['), line.rfind(']')) {
        (Some(open), Some(close)) if close > open + 1 => &line[open + 1..close],
        _ => "",
    };
    if !inner.is_empty() {
        let fill = inner
            .chars()
            .filter(|c| matches!(c, '=' | '#' | '>' | '*' | '-' | '+'))
            .count();
        let blank = inner
            .chars()
            .filter(|c| *c == ' ' || *c == '.' || *c == '_')
            .count();
        if fill >= MIN_BAR_FILL && blank >= 1 {
            return true;
        }
    }
    if !has_percentage(line) {
        return false;
    }
    let mut run = 0usize;
    for ch in line.chars() {
        if matches!(ch, '=' | '#' | '*' | '>') {
            run += 1;
        } else {
            if run >= MIN_BAR_FILL {
                return true;
            }
            run = 0;
        }
    }
    run >= MIN_BAR_FILL
}

/// A percentage readout such as `45%` or ` 4.5%`.
fn has_percentage(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| *byte == b'%' && index > 0 && bytes[index - 1].is_ascii_digit())
}

/// A transfer status line: curl/wget/scp print byte counts with a clock.
fn has_transfer_status(line: &str) -> bool {
    let clocks = line.matches("--:--:--").count();
    if clocks >= 2 {
        return true;
    }
    // `25  1234  25  308  0  0  1234  0  0:00:01` — mostly numbers and one clock.
    let clock = line.matches("0:00:").count();
    if clock >= 2 {
        return true;
    }
    false
}

/// A dot trail such as `Fetching .......` — dots only, since a dash trail is a
/// separator or a diffstat, not progress.
fn has_dot_trail(line: &str) -> bool {
    let mut run = 0usize;
    for ch in line.chars() {
        if ch == '.' {
            run += 1;
        } else {
            if run >= MIN_DOT_TRAIL {
                return true;
            }
            run = 0;
        }
    }
    run >= MIN_DOT_TRAIL
}

/// Braille or ASCII spinner frames at the start of a line.
fn starts_with_spinner(line: &str) -> bool {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some('⠋' | '⠙' | '⠹' | '⠸' | '⠼' | '⠴' | '⠦' | '⠧' | '⠇' | '⠏') => true,
        Some('|') => chars.next().is_none_or(|c| c == ' ' || c == '/' || c == '\\'),
        _ => false,
    }
}

/// A line that starts with one of the wait verbs, optionally behind a
/// timestamp or bracketed prefix.
fn starts_with_progress_verb(line: &str) -> bool {
    let lower = line.to_lowercase();
    let start = lower
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic())
        .map(|(index, _)| index);
    let rest = match start {
        Some(index) => &lower[index..],
        None => return false,
    };
    PROGRESS_VERBS.iter().any(|verb| {
        rest.starts_with(verb)
            && rest[verb.len()..]
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric())
                .unwrap_or(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_progress_lines_collapse_with_count() {
        let mut input = String::new();
        for _ in 0..6 {
            input.push_str("Resolving dependencies...\n");
        }
        input.push_str("added 120 packages in 3s\n");
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        assert_eq!(result.runs_collapsed, 1);
        assert_eq!(result.lines_removed, 5);
        assert!(result.text.contains("Resolving dependencies... [x6]"));
        assert!(result.text.contains("added 120 packages in 3s"));
    }

    #[test]
    fn differing_progress_run_reports_similar_count() {
        let mut input = String::new();
        for pct in 0..10 {
            input.push_str(&format!("Downloading crates [{:=>6}] {pct}%\n", pct));
        }
        input.push_str("finished\n");
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        assert_eq!(result.runs_collapsed, 1);
        assert!(result.text.contains("[+9 similar lines]"), "{}", result.text);
    }

    #[test]
    fn run_ending_at_an_error_keeps_the_error_and_the_line_after_it() {
        let mut input = String::new();
        for pct in 0..6 {
            input.push_str(&format!("Downloading [{:=>6}] {pct}%\n", pct));
        }
        input.push_str("Downloading [==>   ] 60% failed: connection reset\n");
        input.push_str("Downloading [==>   ] 70%\n");
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        // The failure line contains a signal word, so it starts a new run and
        // both it and everything on its far side survive verbatim.
        assert!(result.text.contains("connection reset"), "{}", result.text);
        assert!(result.text.contains("70%"), "{}", result.text);
        assert!(
            result.text.contains("[+5 similar lines]"),
            "{}",
            result.text
        );
        for line in result.text.lines() {
            if line.ends_with("similar lines]") {
                continue;
            }
            assert!(input.contains(line), "output line not in input: {line}");
        }
    }

    #[test]
    fn json_is_refused() {
        let input = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3,\n  \"d\": 4,\n  \"e\": 5,\n  \"f\": 6,\n  \"g\": 7\n}\n";
        let result = collapse_noise_runs(input, 2).expect("stage runs");
        assert_eq!(result.text, input);
        assert_eq!(result.runs_collapsed, 0);
    }

    #[test]
    fn diff_payload_is_refused() {
        let mut input = String::from("diff --git a/x b/x\n");
        for _ in 0..10 {
            input.push_str("Downloading crates ...\n");
        }
        let result = collapse_noise_runs(&input, 2).expect("stage runs");
        assert_eq!(result.text, input);
    }

    #[test]
    fn short_input_is_left_alone() {
        let input = "Downloading a\nDownloading b\nDownloading c\n";
        let result = collapse_noise_runs(input, 2).expect("stage runs");
        assert_eq!(result.text, input);
    }

    #[test]
    fn redraw_bar_in_a_three_line_capture_still_collapses() {
        // The shape a real `curl` progress meter takes: two header lines and one
        // line carrying every frame behind `\r`.
        let mut input = String::from("  % Total    % Received % Xferd  Average Speed\n");
        input.push_str("                                 Dload  Upload\n");
        for pct in [0, 12, 37, 68, 100] {
            input.push_str(&format!(
                "\r{pct:3}  755k  {pct:3}  {}  0  0  25170  0  0:00:30  0:00:29 --:--:-- 28583",
                pct * 7
            ));
        }
        input.push('\n');
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        assert_eq!(result.frames_collapsed, 1);
        assert!(result.text.contains("[x5 frames]"), "{}", result.text);
        assert!(!result.text.contains('\r'));
        assert!(result.text.contains("100  755k  100"), "{}", result.text);
    }

    #[test]
    fn carriage_return_redraws_keep_the_visible_frame() {
        let mut input = String::new();
        for frag in 0..5 {
            input.push_str(&format!(" 25  1234   25   308    0    0   1234      {frag} --:--:--\r"));
        }
        input.push_str("100  1234  100  1234    0    0   3210      0  0:00:01  0:00:01\n");
        for _ in 0..8 {
            input.push_str("unrelated output line\n");
        }
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        assert_eq!(result.frames_collapsed, 1);
        assert!(
            result.text.contains("100  1234  100  1234"),
            "{}",
            result.text
        );
        assert!(result.text.contains("[x6 frames]"), "{}", result.text);
        assert!(!result.text.contains('\r'));
    }

    #[test]
    fn error_frame_is_not_dropped() {
        let line = "  10% [==>   ]\r  20% [===>  ]\r error: checksum mismatch\n";
        let mut input = String::from(line);
        for _ in 0..8 {
            input.push_str("unrelated output line\n");
        }
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        assert_eq!(result.frames_collapsed, 0);
        assert!(result.text.contains("checksum mismatch"));
    }

    #[test]
    fn every_kept_line_appears_in_the_input() {
        let mut input = String::new();
        for pct in 0..12 {
            input.push_str(&format!("Installing [{:=>8}] {pct}%\n", pct));
        }
        input.push_str("src/main.rs:12: unexpected token\n");
        input.push_str("real output line\n");
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        for line in result.text.lines() {
            if line.ends_with("similar lines]") || line.ends_with("frames]") {
                continue;
            }
            assert!(input.contains(line), "output line not in input: {line}");
        }
    }

    #[test]
    fn plain_log_lines_are_not_noise() {
        let input = "2026-11-01T12:00:00Z INFO handler=orders status=200\n".repeat(12);
        let result = collapse_noise_runs(&input, 4).expect("stage runs");
        assert_eq!(result.runs_collapsed, 0, "{}", result.text);
        assert_eq!(result.text, input);
    }

    #[test]
    fn section_headers_and_diffstats_are_not_noise() {
        let mut input = String::new();
        for name in ["platform darwin", "rootdir /repo", "plugins xdist", "cachedir .pytest"] {
            input.push_str(&format!("============================= {name} =============================\n"));
        }
        for file in ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"] {
            input.push_str(&format!(" {file} | 12 ++++++------\n"));
        }
        input.push_str("----------\n");
        input.push_str("----------\n");
        input.push_str("----------\n");
        input.push_str("----------\n");
        let result = collapse_noise_runs(&input, 2).expect("stage runs");
        assert_eq!(result.runs_collapsed, 0, "{}", result.text);
        assert_eq!(result.text, input);
    }
}
