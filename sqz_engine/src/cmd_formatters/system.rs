pub fn format_ls(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= 20 { return output.to_string(); }

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("total") { continue; }
        if trimmed.starts_with('d') || trimmed.ends_with('/') {
            dirs.push(trimmed.split_whitespace().last().unwrap_or(trimmed).to_string());
        } else {
            files.push(trimmed.split_whitespace().last().unwrap_or(trimmed).to_string());
        }
    }

    let mut result = Vec::new();
    if !dirs.is_empty() {
        result.push(format!("dirs({}): {}", dirs.len(), dirs.join(", ")));
    }
    if files.len() > 10 {
        result.push(format!("files({}): {}, ...+{}", files.len(),
            files[..5].join(", "), files.len() - 5));
    } else if !files.is_empty() {
        result.push(format!("files({}): {}", files.len(), files.join(", ")));
    }
    result.join("\n")
}

pub fn format_find(output: &str) -> String {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= 20 { return output.to_string(); }

    let mut by_dir: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for line in &lines {
        let path = std::path::Path::new(line.trim());
        let parent = path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        by_dir.entry(parent).or_default().push(name);
    }

    let mut result = Vec::new();
    result.push(format!("{} files found:", lines.len()));
    for (dir, files) in &by_dir {
        if files.len() > 5 {
            result.push(format!("  {}/ ({} files)", dir, files.len()));
        } else {
            result.push(format!("  {}/ {}", dir, files.join(", ")));
        }
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ls_short_passthrough() {
        let output = "file1.rs\nfile2.rs\n";
        assert_eq!(format_ls(output), output);
    }
}
