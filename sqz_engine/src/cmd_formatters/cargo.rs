use super::test_output::format_test_failures;

pub fn format_cargo(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "test" | "nextest" => Some(format_test_failures(output)),
        "build" | "check" => Some(format_cargo_build(output)),
        "clippy" => Some(super::lint::format_lint(output)),
        _ => None,
    }
}

fn format_cargo_build(output: &str) -> String {
    let errors: Vec<&str> = output.lines()
        .filter(|l| l.starts_with("error") || l.contains("error[E") || l.starts_with("warning"))
        .collect();

    if errors.is_empty() {
        for line in output.lines().rev() {
            if line.contains("Finished") || line.contains("Compiling") {
                return line.trim().to_string();
            }
        }
        return "ok".to_string();
    }

    let mut grouped: Vec<String> = Vec::new();

    for line in output.lines() {
        if line.starts_with("error") || line.contains("error[E") {
            grouped.push(line.to_string());
        } else if line.trim().starts_with("-->") {
            grouped.push(format!("  {}", line.trim()));
        }
    }

    if grouped.is_empty() {
        return output.to_string();
    }

    format!("ERRORS: {}\n{}", errors.len(), grouped.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cargo_build_success() {
        let output = "   Compiling sqz v1.0.0\n    Finished dev [unoptimized + debuginfo] target(s) in 2.5s\n";
        let result = format_cargo_build(output);
        assert!(result.contains("Finished"));
    }

    #[test]
    fn test_cargo_build_errors() {
        let output = "error[E0308]: mismatched types\n  --> src/main.rs:10:5\n";
        let result = format_cargo_build(output);
        assert!(result.contains("ERRORS: 1"));
        assert!(result.contains("E0308"));
    }
}
