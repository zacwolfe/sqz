pub const DEFAULT_MAX_TOKENS: u32 = 8000;

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) { i -= 1; }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) { i += 1; }
    i
}

/// Caps a string to a maximum number of tokens.
/// Returns None if the text is already within the limit or cannot be meaningfully split.
pub fn cap_to_tokens<F>(text: &str, max_tokens: u32, count: F) -> Option<String>
where
    F: Fn(&str) -> u32,
{
    if max_tokens < 64 || count(text) <= max_tokens {
        return None;
    }

    let mut current_max = max_tokens;
    for _ in 0..3 {
        let marker_reserve = 64;
        if current_max <= marker_reserve {
            return None;
        }

        let available = current_max - marker_reserve;
        let head_budget = available / 2;
        let tail_budget = available - head_budget;

        // Head: largest boundary with count(prefix) <= head_budget
        let (mut lo, mut hi) = (0usize, text.len());
        while lo < hi {
            let mid = floor_boundary(text, (lo + hi + 1) / 2);
            if mid <= lo { break; }
            if count(&text[..mid]) <= head_budget { lo = mid; } else { hi = mid - 1; }
        }
        let head_end = floor_boundary(text, lo);

        // Tail: smallest boundary with count(suffix) <= tail_budget
        let (mut lo, mut hi) = (0usize, text.len());
        while lo < hi {
            let mid = ceil_boundary(text, (lo + hi) / 2);
            if mid >= hi { break; }
            if count(&text[mid..]) <= tail_budget { hi = mid; } else { lo = mid + 1; }
        }
        let tail_start = ceil_boundary(text, hi);

        if head_end == 0 || tail_start == text.len() || head_end >= tail_start {
            current_max -= 10;
            continue;
        }

        let omitted_count = text[head_end..tail_start].chars().count();
        let marker = format!(
            "\n[... {} chars omitted: output exceeded the {}-token cap; kept first and last halves ...]\n",
            omitted_count, max_tokens
        );

        let result = format!("{}{}{}", &text[..head_end], marker, &text[tail_start..]);

        if count(&result) <= current_max + 8 {
            return Some(result);
        }

        current_max -= 10;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_counter(s: &str) -> u32 {
        (s.chars().count() as u32 / 4) + 1
    }

    #[test]
    fn test_short_text_unchanged() {
        let text = "Short text";
        assert!(cap_to_tokens(text, 100, mock_counter).is_none());
    }

    #[test]
    fn test_long_text_caps() {
        let text = "a".repeat(1000);
        let capped = cap_to_tokens(&text, 100, mock_counter).expect("Should cap");
        assert!(capped.contains("chars omitted"));
        assert!(mock_counter(&capped) <= 108);
    }

    #[test]
    fn test_multibyte_chars() {
        let text = "🚀".repeat(1000);
        let capped = cap_to_tokens(&text, 100, mock_counter).expect("Should cap");
        assert!(capped.contains("chars omitted"));
        assert!(capped.starts_with('🚀'));
        assert!(capped.ends_with('🚀'));
    }

    #[test]
    fn test_hard_ceiling() {
        let text = "a".repeat(1000);
        let capped = cap_to_tokens(&text, 100, mock_counter).expect("Should cap");
        assert!(mock_counter(&capped) <= 108);
    }

    #[test]
    fn test_large_output_performance() {
        // 1.3MB string. If this is quadratic, it will take seconds.
        // With binary search, it should be near-instant.
        let text = "a".repeat(1_300_000);
        let start = std::time::Instant::now();
        let capped = cap_to_tokens(&text, 8000, |s| (s.len() as u32 / 4) + 1).expect("Should cap");
        let duration = start.elapsed();
        
        assert!(duration.as_millis() < 100, "Capping 1.3MB took too long: {:?}. Binary search likely broken.", duration);
        assert!(capped.contains("chars omitted"));
    }
}
