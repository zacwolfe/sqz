/// Run-Length Encoding (RLE) for structured repetition patterns.
///
/// Generalizes the condense stage to catch repeated patterns within lines
/// and repeated structured blocks, not just consecutive identical lines.
///
/// RLE is provably optimal for data with runs of identical symbols
/// (Shannon 1948). This implementation works at the token/phrase level
/// rather than the byte level, making it effective for CLI output patterns.

use crate::error::Result;

/// Result of RLE compression.
#[derive(Debug, Clone)]
pub struct RleResult {
    /// The compressed text.
    pub text: String,
    /// Number of runs collapsed.
    pub runs_collapsed: usize,
    /// Tokens saved (estimated).
    pub tokens_saved: u32,
}

/// Compress text using token-level run-length encoding.
///
/// Detects repeated phrases/tokens and replaces runs with compact notation:
/// `token [×N]` where N is the repeat count.
pub fn rle_compress(text: &str, min_run_length: usize) -> Result<RleResult> {
    rle_compress_with_original(text, None, min_run_length)
}

pub(crate) fn rle_compress_with_original(
    text: &str,
    original: Option<&str>,
    min_run_length: usize,
) -> Result<RleResult> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return Ok(RleResult {
            text: text.to_string(),
            runs_collapsed: 0,
            tokens_saved: 0,
        });
    }

    let mut original_runs = original.map(line_runs).unwrap_or_default().into_iter();
    let mut output = Vec::new();
    let mut runs_collapsed = 0usize;
    let mut tokens_saved = 0u32;
    let mut i = 0;

    while i < lines.len() {
        // Try to find a run of identical lines (lossless — safe to collapse)
        let mut run_len = 1;
        while i + run_len < lines.len() && lines[i] == lines[i + run_len] {
            run_len += 1;
        }
        let count = match original_runs.next() {
            Some((line, n)) if line == lines[i] => n,
            _ => run_len,
        };

        if run_len >= min_run_length {
            // Collapse the run of identical lines. Lossless: N copies of the
            // same text can be recovered from "{text} [×N]".
            output.push(format!("{} [×{}]", lines[i], count));
            runs_collapsed += 1;
            let line_tokens = (lines[i].len() as u32 + 3) / 4;
            tokens_saved += line_tokens * (run_len as u32 - 1);
            i += run_len;
        } else {
            // Pattern-run detection (lines with shared word-prefix but
            // different suffixes) was removed because it was LOSSY: it
            // replaced the varying parts with "{count} unique values",
            // discarding real filenames from `ls -l` output. See Reddit
            // report #1 and the test_ls_output_preserves_all_filenames
            // regression test. Exact duplicates above are still collapsed.
            output.extend(lines[i..i + run_len].iter().map(|line| line.to_string()));
            i += run_len;
        }
    }

    let trailing_newline = text.ends_with('\n');
    let mut result = output.join("\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Ok(RleResult {
        text: result,
        runs_collapsed,
        tokens_saved,
    })
}

fn line_runs(text: &str) -> Vec<(&str, usize)> {
    let mut runs: Vec<(&str, usize)> = Vec::new();
    for line in text.lines() {
        match runs.last_mut() {
            Some((prev, n)) if *prev == line => *n += 1,
            _ => runs.push((line, 1)),
        }
    }
    runs
}

// ── Log-template compaction ──────────────────────────────────────────────────

/// Result of log-template compaction.
#[derive(Debug, Clone)]
pub struct LogTemplateResult {
    pub text: String,
    pub runs_collapsed: usize,
    pub tokens_saved: u32,
}

/// Collapse consecutive lines that are identical except for VOLATILE spans:
/// timestamps, durations, percentages. What exact RLE (identical lines only)
/// leaves on the table in real logs is mostly timestamp spam.
///
/// Deliberately narrow — the predecessor of this stage (pattern-run
/// detection) was removed for eating `ls` filenames. Rules:
/// - Only timestamps, durations, and percentages count as volatile.
///   Hex ids, UUIDs, filenames, and plain integers make lines DIFFERENT.
/// - Only consecutive lines collapse, and the first line stays verbatim.
/// - The marker names what varied, so the loss is explicit.
pub fn log_template_collapse(text: &str, min_run_length: usize) -> Result<LogTemplateResult> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < min_run_length {
        return Ok(LogTemplateResult {
            text: text.to_string(),
            runs_collapsed: 0,
            tokens_saved: 0,
        });
    }

    let templates: Vec<String> = lines.iter().map(|l| mask_volatiles(l)).collect();

    let mut output = Vec::new();
    let mut runs_collapsed = 0usize;
    let mut tokens_saved = 0u32;
    let mut i = 0;

    while i < lines.len() {
        let mut run_len = 1;
        // A run requires the same template AND that masking actually did
        // something (otherwise identical lines belong to exact RLE).
        while i + run_len < lines.len()
            && templates[i + run_len] == templates[i]
            && templates[i] != lines[i]
            && lines[i + run_len] != lines[i]
        {
            run_len += 1;
        }

        if run_len >= min_run_length {
            output.push(lines[i].to_string());
            output.push(format!(
                "[×{} similar lines; only timestamps/durations vary]",
                run_len - 1
            ));
            runs_collapsed += 1;
            for line in &lines[i + 1..i + run_len] {
                tokens_saved += (line.len() as u32).div_ceil(4);
            }
            i += run_len;
        } else {
            output.push(lines[i].to_string());
            i += 1;
        }
    }

    let trailing_newline = text.ends_with('\n');
    let mut result = output.join("\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Ok(LogTemplateResult {
        text: result,
        runs_collapsed,
        tokens_saved,
    })
}

/// Mask volatile spans (timestamps, durations, percentages) with a
/// placeholder. Everything else — hex ids, UUIDs, paths, plain integers —
/// is preserved so identifier-bearing lines never share a template.
fn mask_volatiles(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for token in line.split_inclusive(|c: char| c.is_whitespace()) {
        let (word, ws) = match token.find(|c: char| c.is_whitespace()) {
            Some(idx) => token.split_at(idx),
            None => (token, ""),
        };
        let stripped = word.trim_matches(|c: char| "[]()<>,".contains(c));
        if is_volatile(stripped) {
            out.push_str(&word.replace(stripped, "‹›"));
        } else {
            out.push_str(word);
        }
        out.push_str(ws);
    }
    out
}

fn is_volatile(token: &str) -> bool {
    if token.len() < 2 {
        return false;
    }
    // Durations: 123ms, 4.5s, 12µs, 3m20s
    for suffix in ["ms", "µs", "us", "ns", "s", "m"] {
        if let Some(num) = token.strip_suffix(suffix) {
            if !num.is_empty()
                && num.chars().all(|c| c.is_ascii_digit() || c == '.')
                && num.chars().any(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
    }
    // Percentages: 45%, 99.9%
    if let Some(num) = token.strip_suffix('%') {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return true;
        }
    }
    // Clock times and dates: 12:34:56(.789), 2026-08-15, 2026-08-15T12:34:56Z
    let digits = token.chars().filter(|c| c.is_ascii_digit()).count();
    if digits >= 4
        && token
            .chars()
            .all(|c| c.is_ascii_digit() || ":-.TZ+".contains(c))
        && (token.contains(':') || token.matches('-').count() >= 2)
    {
        return true;
    }
    false
}

// ── Indexed-row compaction ───────────────────────────────────────────────────

/// Result of indexed-row compaction.
#[derive(Debug, Clone)]
pub struct IndexedRowResult {
    pub text: String,
    pub runs_collapsed: usize,
    pub rows_collapsed: usize,
    pub tokens_saved: u32,
}

/// Collapse consecutive rows that differ only in an index or name field.
///
/// [`log_template_collapse`] masks timestamps, durations, and percentages, so
/// `test tests::case_0 ... ok` and `test tests::case_1 ... ok` never share a
/// template and three hundred such rows survive verbatim. This stage handles
/// that shape: rows with the same token count that differ in one token, or in
/// two tokens that share a non-digit prefix and both end in digits (`t_7`
/// alongside `col_7`, and still true for `t_12` alongside `col_12`).
///
/// Safety rules, all required:
/// - First and last row of a run stay verbatim, so the reader sees where the
///   run starts and where it ends.
/// - The marker names how many rows went missing and which field varied, so
///   the loss is declared rather than silent.
/// - A line containing a signal word (error, panic, failed, assert, ...) is
///   never collapsed and breaks the run, so a run cannot swallow a failure.
/// - Diff payload never collapses: `diff --git`, `@@`, or `+++`/`---` anywhere
///   in the buffer disables the stage, because in a diff the changed lines are
///   the content.
/// - A run needs at least three distinct values, so two rows flapping between
///   two names do not become a "range".
/// - Differing tokens carrying `/`, `=`, or `|` block the run: that is a path,
///   an assignment, or a table cell, not an index.
pub fn collapse_indexed_rows(text: &str, min_run_length: usize) -> Result<IndexedRowResult> {
    let identity = || IndexedRowResult {
        text: text.to_string(),
        runs_collapsed: 0,
        rows_collapsed: 0,
        tokens_saved: 0,
    };

    let lines: Vec<&str> = text.lines().collect();
    let min_run_length = min_run_length.max(3);
    if lines.len() < min_run_length {
        return Ok(identity());
    }
    if lines.iter().any(|line| {
        line.starts_with("diff --git")
            || line.starts_with("@@")
            || line.starts_with("+++ ")
            || line.starts_with("--- ")
    }) {
        return Ok(identity());
    }

    let mut output: Vec<String> = Vec::new();
    let mut runs_collapsed = 0usize;
    let mut rows_collapsed = 0usize;
    let mut tokens_saved = 0u32;

    let mut i = 0;
    while i < lines.len() {
        let mut end = i + 1;
        if !is_signal_row(lines[i]) {
            while end < lines.len()
                && !is_signal_row(lines[end])
                && lines[end] != lines[i]
                && compatible_rows(lines[i], lines[end])
            {
                end += 1;
            }
        }

        let run_len = end - i;
        let omitted = run_len.saturating_sub(2);
        if run_len >= min_run_length
            && omitted > 0
            && distinct_varying_values(&lines[i..end]) >= 3
        {
            let (field, first_value, _) = row_difference(lines[i], lines[i + 1]);
            let (_, _, last_value) = row_difference(lines[i], lines[end - 1]);
            output.push(lines[i].to_string());
            output.push(format!(
                "[×{omitted} rows omitted: {field} spans \"{first_value}\" .. \"{last_value}\", all other fields identical]"
            ));
            output.push(lines[end - 1].to_string());
            runs_collapsed += 1;
            rows_collapsed += omitted;
            for line in &lines[i + 1..end - 1] {
                tokens_saved += (line.len() as u32).div_ceil(4);
            }
            i = end;
        } else {
            output.push(lines[i].to_string());
            i += 1;
        }
    }

    let trailing_newline = text.ends_with('\n');
    let mut result = output.join("\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Ok(IndexedRowResult {
        text: result,
        runs_collapsed,
        rows_collapsed,
        tokens_saved,
    })
}

const INDEXED_SIGNAL_WORDS: [&str; 11] = [
    "error",
    "warning",
    "fatal",
    "panic",
    "exception",
    "failed",
    "failure",
    "assert",
    "traceback",
    "backtrace",
    "stack trace",
];

fn is_signal_row(line: &str) -> bool {
    let lower = line.to_lowercase();
    INDEXED_SIGNAL_WORDS.iter().any(|word| lower.contains(word))
}

/// Trailing run of ASCII digits in `token` (empty when it has none).
fn trailing_digits(token: &str) -> &str {
    let bytes = token.as_bytes();
    let mut start = bytes.len();
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    &token[start..]
}

/// The part of `token` before its trailing digit run, and that digit run.
fn split_trailing_digits(token: &str) -> (&str, &str) {
    let digits = trailing_digits(token);
    (&token[..token.len() - digits.len()], digits)
}

/// Whether two rows differ only in an index or name field.
fn compatible_rows(a: &str, b: &str) -> bool {
    let left: Vec<&str> = a.split_whitespace().collect();
    let right: Vec<&str> = b.split_whitespace().collect();
    if left.len() != right.len() || left.is_empty() {
        return false;
    }
    let differing: Vec<usize> = (0..left.len()).filter(|&k| left[k] != right[k]).collect();
    if differing.is_empty() || differing.len() > 2 {
        return false;
    }
    for &k in &differing {
        let barred = |token: &str| token.contains('/') || token.contains('=') || token.contains('|');
        if barred(left[k]) || barred(right[k]) {
            return false;
        }
    }
    if differing.len() == 2 {
        for &k in &differing {
            let (left_prefix, left_digits) = split_trailing_digits(left[k]);
            let (right_prefix, right_digits) = split_trailing_digits(right[k]);
            if left_digits.is_empty()
                || right_digits.is_empty()
                || left_prefix != right_prefix
            {
                return false;
            }
        }
    }
    true
}

/// `(field label, first row's value, second row's value)` for the first
/// differing token of two rows.
fn row_difference(a: &str, b: &str) -> (String, String, String) {
    let left: Vec<&str> = a.split_whitespace().collect();
    let right: Vec<&str> = b.split_whitespace().collect();
    match (0..left.len().min(right.len())).find(|&k| left[k] != right[k]) {
        Some(k) => (
            format!("field {}", k + 1),
            left[k].to_string(),
            right[k].to_string(),
        ),
        None => (String::new(), String::new(), String::new()),
    }
}

/// How many distinct values the varying field takes across a run.
fn distinct_varying_values(rows: &[&str]) -> usize {
    let mut seen: Vec<String> = Vec::new();
    for pair in rows.windows(2) {
        let (_, first, second) = row_difference(pair[0], pair[1]);
        for value in [first, second] {
            if !value.is_empty() && !seen.iter().any(|known| known == &value) {
                seen.push(value);
            }
        }
    }
    seen.len()
}

// ── Sliding Window Dedup ──────────────────────────────────────────────────────

/// Token-level sliding window deduplication.
///
/// Finds repeated substrings within a text using a sliding window approach.
/// When the same phrase appears multiple times, subsequent occurrences are
/// replaced with back-references: `[→L{line}]`.
///
/// This catches repeated substrings that aren't line-aligned — e.g., the
/// same error message appearing in two different stack frames.
pub fn sliding_window_dedup(text: &str, min_match_words: usize) -> Result<SlidingWindowResult> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return Ok(SlidingWindowResult {
            text: text.to_string(),
            dedup_count: 0,
            tokens_saved: 0,
        });
    }

    // Build a phrase index: map each n-word phrase to its first occurrence line
    let mut phrase_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut output = Vec::new();
    let mut dedup_count = 0usize;
    let mut tokens_saved = 0u32;

    for (line_idx, &line) in lines.iter().enumerate() {
        let words: Vec<&str> = line.split_whitespace().collect();

        // Check if this line (as a phrase) was seen before
        let phrase = words.join(" ");
        if phrase.len() >= min_match_words * 3 {
            if let Some(&first_line) = phrase_index.get(&phrase) {
                // This exact line appeared before — replace with back-reference
                output.push(format!("[→L{}]", first_line + 1));
                dedup_count += 1;
                tokens_saved += (phrase.len() as u32 + 3) / 4;
                continue;
            }
        }

        // Index this line's phrase
        if phrase.len() >= min_match_words * 3 {
            phrase_index.insert(phrase, line_idx);
        }
        output.push(line.to_string());
    }

    let trailing_newline = text.ends_with('\n');
    let mut result = output.join("\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Ok(SlidingWindowResult {
        text: result,
        dedup_count,
        tokens_saved,
    })
}

/// Result of sliding window deduplication.
#[derive(Debug, Clone)]
pub struct SlidingWindowResult {
    pub text: String,
    pub dedup_count: usize,
    pub tokens_saved: u32,
}

// ── Identical-change diff folding ────────────────────────────────────────────

/// Result of cross-file diff folding.
#[derive(Debug, Clone)]
pub struct DiffFoldResult {
    pub text: String,
    /// Files whose body was replaced by a reference line (the exemplar stays).
    pub files_folded: usize,
    /// Payload lines removed across those files.
    pub payload_lines_folded: usize,
    pub tokens_saved: u32,
}

/// Fold a git diff in which many files carry the same change.
///
/// A mechanical refactor shows up as sixty files with one identical hunk each;
/// every byte of the payload is repeated, and the only per-file information the
/// reader needs is where the change landed. This stage keeps the first file
/// verbatim as the exemplar, then replaces each other file in the group with a
/// single reference line carrying its path and its `@@` headers.
///
/// Safety rules, all required:
/// - Only a buffer of `diff --git` sections is touched.
/// - A body is only folded when it is byte-identical to the exemplar's body
///   (digits included), so no digit is ever masked or inferred.
/// - The body must be at least `min_payload_lines` lines, so one-line changes
///   across files are not re-laid-out for nothing.
/// - The declaration line states how many files were folded and that the body
///   is shown once.
/// - Every file path and every `@@` header survives verbatim.
pub fn collapse_identical_diff_files(
    text: &str,
    min_files: usize,
    min_payload_lines: usize,
) -> Result<Option<DiffFoldResult>> {
    let min_files = min_files.max(2);
    if !text.contains("diff --git ") {
        return Ok(None);
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut starts: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("diff --git ") {
            starts.push(i);
        }
    }
    if starts.len() < min_files {
        return Ok(None);
    }

    struct Section<'a> {
        start: usize,
        end: usize,
        path: String,
        body: Vec<&'a str>,
        hunks: Vec<&'a str>,
        unsafe_metadata: bool,
    }

    let mut sections: Vec<Section> = Vec::with_capacity(starts.len());
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(lines.len());
        let expanded = lines[start..end].join("\n");
        let path = diff_section_path(&expanded).unwrap_or_else(|| format!("file{}", n));
        let mut body: Vec<&str> = Vec::new();
        let mut hunks: Vec<&str> = Vec::new();
        let mut unsafe_metadata = false;
        for line in &lines[start..end] {
            if line.starts_with("@@") {
                hunks.push(line);
            } else if is_diff_payload_line(line) {
                body.push(line);
            } else if !is_plain_diff_metadata(line) {
                // Mode changes, renames, copies, and binary patches carry
                // information that is not in the payload. Folding the section
                // would drop it, so a group with any such line stays verbatim.
                unsafe_metadata = true;
            }
        }
        sections.push(Section { start, end, path, body, hunks, unsafe_metadata });
    }

    // Group section indices by body text, preserving first-appearance order.
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (idx, section) in sections.iter().enumerate() {
        if section.body.len() < min_payload_lines || section.hunks.is_empty() {
            continue;
        }
        if section.unsafe_metadata {
            continue;
        }
        let key = section.body.join("\n");
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(idx),
            None => groups.push((key, vec![idx])),
        }
    }

    let mut folded: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut folded_groups: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (_key, members) in &groups {
        if members.len() < min_files {
            continue;
        }
        let exemplar = members[0];
        for &member in &members[1..] {
            folded.insert(member, exemplar);
        }
        folded_groups.insert(exemplar, members.len());
    }

    if folded.is_empty() {
        return Ok(None);
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut files_folded = 0usize;
    let mut payload_lines_folded = 0usize;
    for (idx, section) in sections.iter().enumerate() {
        // Text before the first `diff --git` line is preserved by the caller
        // path only; sections cover the whole buffer here.
        if idx == 0 && section.start > 0 {
            out.extend(lines[..section.start].iter().map(|l| (*l).to_string()));
        }
        if folded.contains_key(&idx) {
            // Both diff path forms (`a/x`, `b/x`) stay so a reader (or a
            // verification pass) still sees the file as the diff header names
            // it, and every hunk header is preserved verbatim.
            out.push(format!(
                "  a/{} b/{} {}",
                section.path,
                section.path,
                section.hunks.join(" ")
            ));
            files_folded += 1;
            payload_lines_folded += section.body.len();
            continue;
        }
        if let Some(&group) = folded_groups.get(&idx) {
            out.push(format!(
                "[identical change: body shown once, {group} files carry it; the rest are \
                 listed as path + hunk header]"
            ));
        }
        out.extend(lines[section.start..section.end].iter().map(|l| (*l).to_string()));
    }

    let mut text_out = out.join("\n");
    if text.ends_with('\n') {
        text_out.push('\n');
    }
    let tokens_saved = ((text.len().saturating_sub(text_out.len())) / 4) as u32;
    Ok(Some(DiffFoldResult {
        text: text_out,
        files_folded,
        payload_lines_folded,
        tokens_saved,
    }))
}

/// `b/<path>` from a `diff --git a/<path> b/<path>` line.
fn diff_section_path(section: &str) -> Option<String> {
    let first = section.lines().next()?;
    let rest = first.strip_prefix("diff --git ")?;
    let after_b = rest.rsplit(" b/").next()?;
    if after_b.is_empty() || after_b == rest {
        return None;
    }
    Some(after_b.to_string())
}

/// Whether a line belongs to a hunk body rather than to file metadata.
fn is_diff_payload_line(line: &str) -> bool {
    if line.starts_with("+++ ") || line.starts_with("--- ") {
        return false;
    }
    line.starts_with(' ')
        || line.starts_with('+')
        || line.starts_with('-')
        || line.starts_with("\\ ")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Whether a metadata line is one folding may drop without losing meaning.
///
/// `diff --git`, `index`, and the `---`/`+++` file markers only name the file,
/// which the reference line already carries. Anything else (mode changes,
/// renames, copies, binary patches) changes what the patch does.
fn is_plain_diff_metadata(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── log-template compaction ───────────────────────────────────────

    #[test]
    fn template_collapses_timestamp_spam() {
        let input = "\
12:00:01 worker heartbeat ok in 12ms
12:00:02 worker heartbeat ok in 14ms
12:00:03 worker heartbeat ok in 11ms
12:00:04 worker heartbeat ok in 13ms
done\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.runs_collapsed, 1);
        assert!(result.text.contains("12:00:01 worker heartbeat ok in 12ms"), "first line verbatim");
        assert!(result.text.contains("[×3 similar lines; only timestamps/durations vary]"));
        assert!(!result.text.contains("12:00:03"), "later volatiles collapsed");
        assert!(result.text.ends_with("done\n"));
    }

    #[test]
    fn template_never_collapses_identifier_bearing_lines() {
        // Same shape but each line carries a distinct hex id: these are the
        // `ls`-filenames class of lines that the removed pattern-run stage
        // used to eat. They must all survive.
        let input = "\
12:00:01 pushed commit deadbeef01 to origin
12:00:02 pushed commit cafebabe02 to origin
12:00:03 pushed commit 0ddba11f03 to origin
12:00:04 pushed commit feedface04 to origin\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.runs_collapsed, 0, "{}", result.text);
        for id in ["deadbeef01", "cafebabe02", "0ddba11f03", "feedface04"] {
            assert!(result.text.contains(id));
        }
    }

    #[test]
    fn template_never_collapses_distinct_filenames() {
        let input = "\
-rw-r--r-- 1 u staff 120 Aug 15 12:00 alpha.rs
-rw-r--r-- 1 u staff 340 Aug 15 12:01 beta.rs
-rw-r--r-- 1 u staff 560 Aug 15 12:02 gamma.rs
-rw-r--r-- 1 u staff 780 Aug 15 12:03 delta.rs\n";
        let result = log_template_collapse(input, 3).unwrap();
        for name in ["alpha.rs", "beta.rs", "gamma.rs", "delta.rs"] {
            assert!(result.text.contains(name), "filename lost: {}", result.text);
        }
    }

    #[test]
    fn template_leaves_identical_lines_to_exact_rle() {
        let input = "same line\nsame line\nsame line\nsame line\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.runs_collapsed, 0);
        assert_eq!(result.text, input);
    }

    #[test]
    fn template_short_runs_untouched() {
        let input = "12:00:01 tick\n12:00:02 tick\nother\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.text, input);
    }

    // --- RLE tests ---

    #[test]
    fn test_rle_no_repetition() {
        let result = rle_compress("alpha\nbeta\ngamma\n", 2).unwrap();
        assert_eq!(result.runs_collapsed, 0);
    }

    #[test]
    fn test_rle_exact_repetition() {
        let input = "ok\nok\nok\nok\nok\n";
        let result = rle_compress(input, 2).unwrap();
        assert!(result.runs_collapsed > 0);
        assert!(result.text.contains("[×5]"), "output: {}", result.text);
    }

    #[test]
    fn test_rle_pattern_no_longer_collapses() {
        // Pattern-run collapsing was removed because it was lossy — it
        // replaced varying filename/identifier suffixes with "N unique
        // values", dropping data the LLM needs. Different lines that
        // merely share a prefix must pass through unchanged.
        let input = "Compiling foo v1.0\nCompiling bar v2.0\nCompiling baz v3.0\ndone\n";
        let result = rle_compress(input, 2).unwrap();
        assert_eq!(result.runs_collapsed, 0, "different lines must not be collapsed");
        assert!(result.text.contains("foo"), "filename 'foo' must survive: {}", result.text);
        assert!(result.text.contains("bar"), "filename 'bar' must survive: {}", result.text);
        assert!(result.text.contains("baz"), "filename 'baz' must survive: {}", result.text);
    }

    #[test]
    fn test_rle_ls_l_output_preserves_filenames() {
        // Regression for https://github.com/ojuschugh1/sqz/issues/1
        // ls -l lines share the 'drwxr-xr-x' prefix but have distinct
        // filenames. Pattern-run collapsing used to delete them.
        let input = "drwxr-xr-x  6 user user  192 Apr 18 10:00 packages\n\
                     drwxr-xr-x  3 user user   96 Apr 18 10:00 configuration\n\
                     drwxr-xr-x  4 user user  128 Apr 18 10:00 documentation\n\
                     drwxr-xr-x  2 user user   64 Apr 18 10:00 environment\n";
        let result = rle_compress(input, 2).unwrap();
        for name in &["packages", "configuration", "documentation", "environment"] {
            assert!(result.text.contains(name),
                "filename '{}' must be preserved — got:\n{}", name, result.text);
        }
    }

    #[test]
    fn test_rle_mixed_content() {
        let input = "header\ntest a ... ok\ntest b ... ok\ntest c ... ok\nfooter\n";
        let result = rle_compress(input, 2).unwrap();
        assert!(result.text.contains("header"));
        assert!(result.text.contains("footer"));
    }

    #[test]
    fn test_rle_preserves_trailing_newline() {
        let result = rle_compress("a\na\na\n", 2).unwrap();
        assert!(result.text.ends_with('\n'));
    }

    #[test]
    fn test_rle_empty_input() {
        let result = rle_compress("", 2).unwrap();
        assert_eq!(result.text, "");
        assert_eq!(result.runs_collapsed, 0);
    }

    #[test]
    fn test_rle_single_line() {
        let result = rle_compress("hello\n", 2).unwrap();
        assert_eq!(result.runs_collapsed, 0);
    }

    #[test]
    fn test_rle_min_run_length_respected() {
        let input = "ok\nok\ndone\n";
        let result_2 = rle_compress(input, 2).unwrap();
        let result_3 = rle_compress(input, 3).unwrap();
        assert!(result_2.runs_collapsed > 0, "run of 2 should collapse with min=2");
        assert_eq!(result_3.runs_collapsed, 0, "run of 2 should NOT collapse with min=3");
    }

    #[test]
    fn test_rle_counts_runs_of_the_original() {
        let original = format!("x\nx\n{}b\n{}", "a\n".repeat(10), "c\n".repeat(3));
        let condensed = "x\nx\na\na\na\nb\nc\nc\nc\n";
        let result = rle_compress_with_original(condensed, Some(&original), 3).unwrap();
        assert_eq!(result.text, "x\nx\na [×10]\nb\nc [×3]\n");
        assert_eq!(rle_compress(condensed, 3).unwrap().text, "x\nx\na [×3]\nb\nc [×3]\n");
    }

    // --- Sliding window dedup tests ---

    #[test]
    fn test_sliding_window_no_duplicates() {
        let result = sliding_window_dedup("line 1\nline 2\nline 3\n", 3).unwrap();
        assert_eq!(result.dedup_count, 0);
    }

    #[test]
    fn test_sliding_window_exact_duplicate_lines() {
        let input = "error: connection refused at port 8080\nsome other line\nerror: connection refused at port 8080\n";
        let result = sliding_window_dedup(input, 3).unwrap();
        assert!(result.dedup_count > 0, "should detect duplicate line");
        assert!(result.text.contains("[→L1]"), "output: {}", result.text);
    }

    #[test]
    fn test_sliding_window_preserves_first_occurrence() {
        let input = "the error message here\nother stuff\nthe error message here\n";
        let result = sliding_window_dedup(input, 3).unwrap();
        // First occurrence should be preserved verbatim
        assert!(result.text.starts_with("the error message here\n"));
    }

    #[test]
    fn test_sliding_window_short_lines_ignored() {
        let input = "ok\nok\nok\n";
        let result = sliding_window_dedup(input, 3).unwrap();
        // "ok" is too short (< min_match_words * 3 = 9 chars)
        assert_eq!(result.dedup_count, 0);
    }

    #[test]
    fn test_sliding_window_empty_input() {
        let result = sliding_window_dedup("", 3).unwrap();
        assert_eq!(result.text, "");
    }

    // ── indexed-row compaction ────────────────────────────────────────

    fn cargo_rows(count: usize) -> String {
        (0..count)
            .map(|n| format!("test tests::case_{n} ... ok\n"))
            .collect()
    }

    #[test]
    fn indexed_rows_collapse_a_passing_test_run() {
        let input = cargo_rows(300);
        let result = collapse_indexed_rows(&input, 4).unwrap();
        assert_eq!(result.runs_collapsed, 1);
        assert_eq!(result.rows_collapsed, 298);
        assert_eq!(result.text.lines().count(), 3, "first row, marker, last row");
        assert!(result.text.contains("test tests::case_0 ... ok"));
        assert!(result.text.contains("test tests::case_299 ... ok"));
        assert!(
            result.text.contains("[×298 rows omitted: field 2 spans \"tests::case_0\" .. \"tests::case_299\""),
            "marker declares the loss and the varying field: {}",
            result.text
        );
        assert!(!result.text.contains("case_150"));
    }

    #[test]
    fn indexed_rows_never_swallow_a_failing_row() {
        let mut input = cargo_rows(300);
        input = input.replace(
            "test tests::case_150 ... ok\n",
            "test tests::case_150 ... FAILED\n",
        );
        let result = collapse_indexed_rows(&input, 4).unwrap();
        assert!(
            result.text.contains("test tests::case_150 ... FAILED"),
            "failing row must survive verbatim: {}",
            result.text
        );
        // The run is split around the failure, so nothing between the two halves
        // is dropped across it.
        for n in [149usize, 151] {
            assert!(
                result.text.contains(&format!("case_{n}")),
                "row {} sits at the run edge and must survive",
                n
            );
        }
    }

    #[test]
    fn indexed_rows_break_a_run_at_an_error_line() {
        let input = "\
report_0 written
report_1 written
report_2 written
report_3 written
error: shard 4 is unavailable
report_5 written
report_6 written
report_7 written
report_8 written
";
        let result = collapse_indexed_rows(input, 3).unwrap();
        assert!(
            result.text.contains("error: shard 4 is unavailable"),
            "a signal line is never folded into a range: {}",
            result.text
        );
        assert_eq!(result.runs_collapsed, 2, "the run splits around the error");
        assert!(!result.text.contains("report_2"), "inner rows still collapse");
    }

    #[test]
    fn indexed_rows_leave_short_runs_alone() {
        let input = cargo_rows(3);
        let result = collapse_indexed_rows(&input, 4).unwrap();
        assert_eq!(result.runs_collapsed, 0);
        assert_eq!(result.text, input);
    }

    #[test]
    fn indexed_rows_leave_multi_field_changes_alone() {
        let input = "\
node_0 has alpha beta gamma
node_1 has delta epsilon zeta
node_2 has eta theta iota
node_3 has kappa lambda mu
";
        let result = collapse_indexed_rows(input, 4).unwrap();
        assert_eq!(result.runs_collapsed, 0, "four varying fields are content");
        assert_eq!(result.text, input);
    }

    #[test]
    fn indexed_rows_collapse_aligned_double_index_rows() {
        let input: String = (0..40)
            .map(|n| format!("ALTER TABLE t_{n} ADD COLUMN col_{n} TEXT;\n"))
            .collect();
        let result = collapse_indexed_rows(&input, 4).unwrap();
        assert_eq!(result.runs_collapsed, 1);
        assert_eq!(result.rows_collapsed, 38);
        assert!(result.text.contains("ALTER TABLE t_0 ADD COLUMN col_0 TEXT;"));
        assert!(result.text.contains("ALTER TABLE t_39 ADD COLUMN col_39 TEXT;"));
    }

    #[test]
    fn indexed_rows_never_touch_diff_payload() {
        let input = "\
diff --git a/src/mod_0.rs b/src/mod_0.rs\nindex 0000000..1111111 100644\n@@ -0,7 +0,9 @@\n";
        let result = collapse_indexed_rows(input, 4).unwrap();
        assert_eq!(result.runs_collapsed, 0);
        assert_eq!(result.text, input);
    }

    // ── identical-change diff folding ─────────────────────────────────

    fn repeated_diff(files: usize) -> String {
        let mut out = String::new();
        for i in 0..files {
            out.push_str(&format!(
                "diff --git a/src/mod_{i}.rs b/src/mod_{i}.rs\n\
                 index abc{i}..def{i} 100644\n\
                 --- a/src/mod_{i}.rs\n\
                 +++ b/src/mod_{i}.rs\n\
                 @@ -{i}0,4 +{i}0,6 @@ impl Mod{i} {{\n\
                 -    let x = compute();\n\
                 +    let mut x = compute().unwrap_or(0);\n\
                 +    log::trace!(\"x={{x}}\");\n\
                 -    return x;\n\
                 +    x\n"
            ));
        }
        out
    }

    #[test]
    fn diff_folding_keeps_paths_and_hunk_headers() {
        let input = repeated_diff(6);
        let result = collapse_identical_diff_files(&input, 3, 4)
            .unwrap()
            .expect("identical bodies must fold");
        assert_eq!(result.files_folded, 5);
        for i in 0..6 {
            assert!(
                result.text.contains(&format!("src/mod_{i}.rs")),
                "lost path mod_{i}: {}",
                result.text
            );
            assert!(
                result.text.contains(&format!("@@ -{i}0,4 +{i}0,6 @@ impl Mod{i} {{")),
                "lost hunk header {i}: {}",
                result.text
            );
        }
        // The body appears once, with its real digits.
        assert_eq!(result.text.matches("let mut x = compute().unwrap_or(0);").count(), 1);
        assert!(result.text.contains("6 files carry it"));
        assert!(result.text.len() < input.len());
    }

    #[test]
    fn diff_folding_needs_identical_bodies() {
        // Make one body differ by a single digit: numbers are never masked.
        let input = repeated_diff(3).replacen("unwrap_or(0)", "unwrap_or(7)", 1);
        let result = collapse_identical_diff_files(&input, 3, 4).unwrap();
        assert!(result.is_none(), "folded files whose bodies differ");
    }

    #[test]
    fn diff_folding_skips_small_groups_and_short_bodies() {
        let two_files = repeated_diff(2);
        assert!(collapse_identical_diff_files(&two_files, 3, 4).unwrap().is_none());
        let long_bodies = repeated_diff(4);
        assert!(
            collapse_identical_diff_files(&long_bodies, 3, 99).unwrap().is_none(),
            "payload shorter than the minimum must not fold"
        );
    }

    #[test]
    fn diff_folding_ignores_non_diff_text() {
        let log = "line one\nline two\nline three\n";
        assert!(collapse_identical_diff_files(log, 3, 4).unwrap().is_none());
    }

    #[test]
    fn diff_folding_refuses_sections_with_mode_or_rename_metadata() {
        // A mode change or a rename is not part of the payload, so folding the
        // section would silently drop it. Those groups stay verbatim.
        let input = repeated_diff(4).replacen(
            "index abc1..def1 100644",
            "old mode 100644\nnew mode 100755",
            1,
        );
        let result = collapse_identical_diff_files(&input, 3, 4)
            .unwrap()
            .expect("the clean sections still fold");
        assert!(
            result.text.contains("new mode 100755"),
            "mode change lost: {}",
            result.text
        );
        // Three clean files folded (one exemplar, two references); the section
        // with metadata is untouched.
        assert_eq!(result.files_folded, 2);
    }

    use proptest::prelude::*;

    proptest! {
        /// RLE never produces output longer than input (excluding the run markers).
        #[test]
        fn prop_rle_tokens_saved_non_negative(
            text in "[a-z]{3,10}(\n[a-z]{3,10}){2,10}\n"
        ) {
            let result = rle_compress(&text, 2).unwrap();
            // tokens_saved is u32, always >= 0
            let _ = result.tokens_saved;
            // Output should not be empty if input isn't
            prop_assert!(!result.text.is_empty());
        }

        /// Sliding window dedup count is non-negative and bounded.
        #[test]
        fn prop_sliding_window_bounded(
            text in "[a-z ]{10,50}(\n[a-z ]{10,50}){2,8}\n"
        ) {
            let result = sliding_window_dedup(&text, 3).unwrap();
            let line_count = text.lines().count();
            prop_assert!(
                result.dedup_count <= line_count,
                "dedup count {} exceeds line count {}",
                result.dedup_count, line_count
            );
        }
    }
}
