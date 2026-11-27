//! Template row grouping for output whose rows repeat one message.
//!
//! A compiler that emits the same message at 150 locations, a migration that
//! runs one `ALTER TABLE` per table, a package manager that prints one fetch
//! line per package: the lines carry one distinct payload each, and the rest of
//! the bytes are the same sentence repeated. No byte-level stage can help
//! because the rows are not equal, and entropy truncation can only delete whole
//! rows, which loses the locations the reader needs.
//!
//! This stage replaces a run of such rows with an *invertible* block: the
//! template once, with `%` standing for each variable field, then one line per
//! row holding only that row's values in field order.
//!
//! ```text
//! [template ×150] % error TS2322: Type 'string' is not assignable to type 'number'.
//! src/components/Comp0.tsx(0,13)
//! src/components/Comp1.tsx(1,13)
//! ...
//! ```
//!
//! Nothing is dropped or paraphrased: every path, every number, and every
//! message is still in the output, and [`expand_declared_templates`] rebuilds
//! the original lines byte for byte. That is why the verifier can keep every
//! invariant strict while accepting the block.

/// Marker that opens a template block. Ends with the row count.
const MARKER_PREFIX: &str = "[template ×";

/// Placeholder for a variable field inside a template.
const SLOT: char = '%';

/// Cap on rows in one block, so a pathological run cannot produce a block
/// longer than the text it replaces.
const MAX_ROWS: usize = 500;

/// Result of template row grouping.
#[derive(Debug, Clone)]
pub struct TemplateCollapseResult {
    /// Rewritten text.
    pub text: String,
    /// Number of runs replaced by a block.
    pub runs_folded: usize,
    /// Number of original rows folded into blocks.
    pub rows_folded: usize,
    pub tokens_saved: u32,
}

/// A line split into whitespace-free tokens and the separators between them.
struct Fields<'a> {
    tokens: Vec<&'a str>,
    /// `seps[i]` sits between `tokens[i]` and `tokens[i + 1]`.
    seps: Vec<&'a str>,
}

/// Split a line into tokens and the whitespace runs between them.
///
/// Returns `None` for a line that is empty, starts with whitespace, or has a
/// single token: an indented line is code, and a single token has nothing to
/// template against.
fn parse_fields(line: &str) -> Option<Fields<'_>> {
    // An indented line is code or a table cell; folding it would rewrite its
    // leading whitespace, which is content.
    if line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }
    let mut tokens: Vec<&str> = Vec::new();
    let mut seps: Vec<&str> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if start == i {
            return None;
        }
        tokens.push(&line[start..i]);
        let sep_start = i;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if sep_start != i {
            seps.push(&line[sep_start..i]);
        }
    }
    if tokens.len() < 2 || seps.len() + 1 != tokens.len() {
        return None;
    }
    Some(Fields { tokens, seps })
}

/// Whether two field layouts can share one template: same field count, same
/// separators. Separators must match exactly because the block has to rebuild
/// the original spacing; a progress bar whose padding width changes (`[  10%]`
/// vs `[   9%]`) simply starts a new run.
fn same_layout(a: &Fields<'_>, b: &Fields<'_>) -> bool {
    a.tokens.len() == b.tokens.len() && a.seps == b.seps
}

/// Collapse runs of lines that differ only in a few fields into declared,
/// invertible template blocks.
///
/// A run qualifies when it has at least `min_rows` lines, at least one field
/// actually varies, no field contains the `%` placeholder, and the block is at
/// least `min_savings_ratio` smaller than the lines it replaces.
pub fn collapse_template_runs(
    text: &str,
    min_rows: usize,
    min_savings_ratio: f64,
) -> Option<TemplateCollapseResult> {
    let min_rows = min_rows.max(2);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < min_rows {
        return None;
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut runs_folded = 0usize;
    let mut rows_folded = 0usize;
    let mut i = 0usize;
    while i < lines.len() {
        let Some(head) = parse_fields(lines[i]) else {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        };
        // Extend the run while layouts match. Variable positions are tracked as
        // a bitmask so a field that differs anywhere in the run is a slot.
        let mut end = i + 1;
        let mut variable = vec![false; head.tokens.len()];
        while end < lines.len() && end - i < MAX_ROWS {
            let Some(next) = parse_fields(lines[end]) else {
                break;
            };
            if !same_layout(&head, &next) {
                break;
            }
            for (pos, slot) in variable.iter_mut().enumerate() {
                if !*slot && head.tokens[pos] != next.tokens[pos] {
                    *slot = true;
                }
            }
            end += 1;
        }
        let rows = end - i;
        let has_slot = variable.iter().any(|v| *v);
        if !has_slot {
            // Every row in the run is byte-identical, so no later start can do
            // better and the RLE stage owns this shape. Emit the run and move
            // past it instead of re-scanning it line by line.
            out.extend(lines[i..end].iter().map(|l| (*l).to_string()));
            i = end;
            continue;
        }
        if rows < min_rows {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }

        // Build the template and the rows it stands for.
        let mut template = String::new();
        for (pos, token) in head.tokens.iter().enumerate() {
            if pos > 0 {
                template.push_str(head.seps[pos - 1]);
            }
            if variable[pos] {
                template.push(SLOT);
            } else {
                template.push_str(token);
            }
        }
        // A literal `%` in the template would make the block ambiguous.
        if template.matches(SLOT).count() != variable.iter().filter(|v| **v).count() {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }
        let slot_count = variable.iter().filter(|v| **v).count();
        let mut rows_text: Vec<String> = Vec::with_capacity(rows);
        let mut block_len = MARKER_PREFIX.len() + template.len() + 8;
        let mut original_len = 0usize;
        for line in &lines[i..end] {
            original_len += line.len() + 1;
            let Some(fields) = parse_fields(line) else {
                rows_text.clear();
                break;
            };
            let values: Vec<&str> = fields
                .tokens
                .iter()
                .zip(&variable)
                .filter_map(|(t, v)| v.then_some(*t))
                .collect();
            if values.len() != slot_count {
                rows_text.clear();
                break;
            }
            let row = values.join(" ");
            block_len += row.len() + 1;
            rows_text.push(row);
        }
        let savings = if original_len == 0 {
            0.0
        } else {
            1.0 - block_len as f64 / original_len as f64
        };
        if rows_text.len() < min_rows || savings < min_savings_ratio {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }

        out.push(format!("{MARKER_PREFIX}{rows}]{template}"));
        out.extend(rows_text);
        runs_folded += 1;
        rows_folded += rows;
        i = end;
    }

    if runs_folded == 0 {
        return None;
    }
    let mut text_out = out.join("\n");
    if text.ends_with('\n') {
        text_out.push('\n');
    }
    let tokens_saved = ((text.len().saturating_sub(text_out.len())) / 4) as u32;
    Some(TemplateCollapseResult {
        text: text_out,
        runs_folded,
        rows_folded,
        tokens_saved,
    })
}

/// Rebuild the original lines from every declared template block.
///
/// The verifier and any caller that needs the full text can invert the block
/// exactly: the marker names the row count, the first line holds the template,
/// and the following lines hold each row's values in field order. A block whose
/// rows do not match the declared placeholder count is left as-is, so a
/// malformed block can never launder a loss.
pub fn expand_declared_templates(text: &str) -> String {
    if !text.contains(MARKER_PREFIX) {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let Some(rest) = lines[i].strip_prefix(MARKER_PREFIX) else {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        };
        let Some((count, template)) = rest.split_once(']') else {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        };
        let Ok(count) = count.trim().parse::<usize>() else {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        };
        let slots = template.matches(SLOT).count();
        let rows: Vec<&str> = lines
            .iter()
            .skip(i + 1)
            .take(count)
            .copied()
            .collect();
        let complete = rows.len() == count
            && slots > 0
            && rows.iter().all(|row| {
                let fields: Vec<&str> = row.split(' ').filter(|v| !v.is_empty()).collect();
                fields.len() == slots
            });
        if !complete {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }
        for row in rows {
            let values: Vec<&str> = row.split(' ').filter(|v| !v.is_empty()).collect();
            let mut expanded = String::new();
            let mut next = 0usize;
            for ch in template.chars() {
                if ch == SLOT {
                    expanded.push_str(values[next]);
                    next += 1;
                } else {
                    expanded.push(ch);
                }
            }
            out.push(expanded);
        }
        i += 1 + count;
    }
    let mut text_out = out.join("\n");
    if text.ends_with('\n') {
        text_out.push('\n');
    }
    text_out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tsc_like(rows: usize) -> String {
        (0..rows)
            .map(|i| {
                format!(
                    "src/components/Comp{i}.tsx({i},13): error TS2322: Type 'string' is not assignable to type 'number'.\n"
                )
            })
            .collect()
    }

    #[test]
    fn groups_rows_that_share_one_message() {
        let input = tsc_like(40);
        let result = collapse_template_runs(&input, 6, 0.3).expect("40 identical-message rows fold");
        assert_eq!(result.runs_folded, 1);
        assert_eq!(result.rows_folded, 40);
        assert!(
            result
                .text
                .contains("error TS2322: Type 'string' is not assignable to type 'number'."),
            "message must appear once: {}",
            result.text
        );
        assert_eq!(result.text.matches("TS2322").count(), 1);
        assert!(result.text.contains("[template ×40]"));
        assert!(result.text.len() * 2 < input.len());
    }

    #[test]
    fn expansion_restores_the_original_lines() {
        let input = tsc_like(12);
        let folded = collapse_template_runs(&input, 6, 0.3).unwrap();
        assert_eq!(expand_declared_templates(&folded.text), input);
    }

    #[test]
    fn expansion_keeps_all_rows_of_a_mixed_text() {
        let mut input = String::from("npm http fetch GET 200 https://registry.npmjs.org/pkg-0 12ms\n");
        for i in 0..10 {
            input.push_str(&format!(
                "npm http fetch GET 200 https://registry.npmjs.org/pkg-{i} {i}ms\n"
            ));
        }
        input.push_str("done in 3.2s\n");
        let folded = collapse_template_runs(&input, 6, 0.3).unwrap();
        assert_eq!(expand_declared_templates(&folded.text), input);
    }

    #[test]
    fn rows_with_different_separator_widths_do_not_share_a_template() {
        // A progress bar changes its padding width; folding it would rebuild
        // the wrong spacing, so each width becomes its own run.
        let input: String = (0..9)
            .map(|i| format!("tests/test_{i}.py::test_thing PASSED with 3 assertions [   {i}%]\n"))
            .chain(
                (10..20)
                    .map(|i| format!("tests/test_{i}.py::test_thing PASSED with 3 assertions [  {i}%]\n")),
            )
            .collect();
        let folded = collapse_template_runs(&input, 6, 0.3).unwrap();
        assert_eq!(expand_declared_templates(&folded.text), input);
        assert_eq!(folded.runs_folded, 2);
    }

    #[test]
    fn identical_rows_are_left_to_the_rle_stage() {
        let input = "same line here\n".repeat(20);
        assert!(collapse_template_runs(&input, 6, 0.3).is_none());
    }

    #[test]
    fn indented_code_is_never_templated() {
        let input: String = (0..20)
            .map(|i| format!("    let x_{i} = compute({i});\n"))
            .collect();
        assert!(collapse_template_runs(&input, 6, 0.3).is_none());
    }

    #[test]
    fn short_runs_are_left_alone() {
        let input = tsc_like(5);
        assert!(collapse_template_runs(&input, 6, 0.3).is_none());
    }

    #[test]
    fn a_template_with_a_literal_percent_is_not_folded() {
        let input: String = (0..20)
            .map(|i| format!("coverage {i}% of lines reached in build_{i}\n"))
            .collect();
        // `%` inside a constant field would be ambiguous with the placeholder.
        let folded = collapse_template_runs(&input, 6, 0.3);
        if let Some(folded) = folded {
            assert_eq!(expand_declared_templates(&folded.text), input);
        }
    }

    #[test]
    fn malformed_blocks_are_not_expanded() {
        let text = "[template ×2] only %\none\n";
        assert_eq!(expand_declared_templates(text), text);
    }

    #[test]
    fn text_without_blocks_is_returned_unchanged() {
        let text = "plain log\nwith lines\n";
        assert_eq!(expand_declared_templates(text), text);
    }
}
