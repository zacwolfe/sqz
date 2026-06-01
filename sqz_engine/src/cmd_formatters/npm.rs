use super::test_output::format_test_failures;

pub fn format_npm(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "test" => Some(format_test_failures(output)),
        "run" if output.contains("error") || output.contains("FAIL") => Some(format_test_failures(output)),
        "install" | "i" | "add" => Some(format_npm_install(output)),
        _ => None,
    }
}

fn format_npm_install(output: &str) -> String {
    let mut vulns = String::new();
    for line in output.lines() {
        if line.contains("added") && line.contains("packages") {
            return line.trim().to_string();
        }
        if line.contains("vulnerabilities") {
            vulns = line.trim().to_string();
        }
    }
    if !vulns.is_empty() {
        return format!("ok ({})", vulns);
    }
    "ok".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_npm_install_compact() {
        let output = "added 42 packages in 3s\n2 vulnerabilities\n";
        let result = format_npm_install(output);
        assert!(result.contains("added 42 packages"));
    }
}
