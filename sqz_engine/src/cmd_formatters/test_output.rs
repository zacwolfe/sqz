use super::truncate::CAP_ERRORS;

pub fn format_test_failures(output: &str) -> String {
    let mut failures = Vec::new();
    let mut summary_line = String::new();
    let mut in_failure = false;
    let mut failure_buf = Vec::new();

    for line in output.lines() {
        if line.starts_with("test result:") || line.starts_with("Tests:") {
            summary_line = line.to_string();
        }
        // Rust: "---- test_name stdout ----" marks failure start
        if line.starts_with("---- ") && line.ends_with(" ----") {
            if !failure_buf.is_empty() {
                failures.push(failure_buf.join("\n"));
                failure_buf.clear();
            }
            in_failure = true;
            failure_buf.push(line.to_string());
            continue;
        }
        if line == "failures:" {
            in_failure = true;
            continue;
        }
        // pytest: "FAILED" lines
        if line.contains("FAILED") || line.contains("FAIL:") {
            failures.push(line.to_string());
        }
        // go test: "--- FAIL:"
        if line.starts_with("--- FAIL:") {
            failures.push(line.to_string());
        }
        if in_failure {
            if line.trim().is_empty() && !failure_buf.is_empty() {
                failures.push(failure_buf.join("\n"));
                failure_buf.clear();
                in_failure = false;
            } else {
                failure_buf.push(line.to_string());
            }
        }
        if line.contains("... FAILED") || line.contains("FAILED") && line.starts_with("test ") {
            if !failures.iter().any(|f| f.contains(line)) {
                failures.push(line.to_string());
            }
        }
    }
    if !failure_buf.is_empty() {
        failures.push(failure_buf.join("\n"));
    }

    if failures.is_empty() {
        if !summary_line.is_empty() {
            return summary_line;
        }
        let total = output.lines().filter(|l| l.contains("... ok") || l.contains("PASSED") || l.contains("passed")).count();
        if total > 0 {
            return format!("ok: {} tests passed", total);
        }
        return output.to_string();
    }

    // Truncate excessive failures
    let display_failures = if failures.len() > CAP_ERRORS {
        let omitted = failures.len() - CAP_ERRORS;
        let mut truncated: Vec<String> = failures[..CAP_ERRORS].to_vec();
        truncated.push(format!("...+{} more failures", omitted));
        truncated
    } else {
        failures
    };

    let mut result = Vec::new();
    if !summary_line.is_empty() {
        result.push(summary_line);
    }
    result.push(format!("FAILURES ({}):", display_failures.len()));
    for f in &display_failures {
        result.push(f.clone());
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_pass() {
        let output = "running 15 tests\ntest a ... ok\ntest b ... ok\ntest result: ok. 15 passed; 0 failed; 0 ignored\n";
        let result = format_test_failures(output);
        assert!(result.contains("ok") || result.contains("passed"));
        assert!(!result.contains("FAILURES"));
    }

    #[test]
    fn test_with_failure() {
        let output = "running 3 tests\ntest a ... ok\ntest b ... FAILED\ntest c ... ok\n\nfailures:\n\n---- b stdout ----\nassertion failed\n\ntest result: FAILED. 2 passed; 1 failed\n";
        let result = format_test_failures(output);
        assert!(result.contains("FAIL"));
    }
}
