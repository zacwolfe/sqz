pub fn format_git(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "status" => Some(format_git_status(output)),
        "log" => Some(format_git_log(output)),
        "diff" => Some(format_git_diff(output)),
        "add" | "commit" | "push" | "pull" | "checkout" | "switch" | "branch" => {
            Some(format_git_short(subcmd.unwrap(), output))
        }
        _ => None,
    }
}

fn format_git_status(output: &str) -> String {
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("new file:") || trimmed.starts_with("modified:") && line.starts_with('\t') {
            staged.push(trimmed.to_string());
        } else if trimmed.starts_with("modified:") || trimmed.starts_with("deleted:") {
            modified.push(trimmed.to_string());
        } else if line.starts_with("\t") && !trimmed.starts_with("(use") {
            if output[..output.find(line).unwrap_or(0)].contains("Untracked files:") {
                untracked.push(trimmed.to_string());
            }
        }
    }

    // Also handle short-format status (git status -s)
    if staged.is_empty() && modified.is_empty() && untracked.is_empty() {
        let mut short_staged = Vec::new();
        let mut short_modified = Vec::new();
        let mut short_untracked = Vec::new();
        for line in output.lines() {
            if line.len() < 3 { continue; }
            let (idx, rest) = (line.get(..2), line.get(3..));
            if let (Some(idx), Some(rest)) = (idx, rest) {
                match idx.trim() {
                    "M" | "A" | "D" | "R" => short_staged.push(format!("{} {}", idx.trim(), rest)),
                    "??" => short_untracked.push(rest.to_string()),
                    _ if idx.contains('M') => short_modified.push(format!("M {}", rest)),
                    _ => {}
                }
            }
        }
        if !short_staged.is_empty() || !short_modified.is_empty() || !short_untracked.is_empty() {
            staged = short_staged;
            modified = short_modified;
            untracked = short_untracked;
        }
    }

    if staged.is_empty() && modified.is_empty() && untracked.is_empty() {
        if output.contains("nothing to commit") {
            return "clean".to_string();
        }
        return output.to_string();
    }

    let mut result = Vec::new();
    if !staged.is_empty() {
        result.push(format!("staged({}): {}", staged.len(), staged.join(", ")));
    }
    if !modified.is_empty() {
        result.push(format!("modified({}): {}", modified.len(), modified.join(", ")));
    }
    if !untracked.is_empty() {
        if untracked.len() > 5 {
            result.push(format!("untracked({}): {}, ...+{}", untracked.len(),
                untracked[..3].join(", "), untracked.len() - 3));
        } else {
            result.push(format!("untracked({}): {}", untracked.len(), untracked.join(", ")));
        }
    }
    result.join("\n")
}

fn format_git_log(output: &str) -> String {
    let mut commits = Vec::new();
    let mut current_hash = String::new();
    let mut current_subject = String::new();

    for line in output.lines() {
        if line.starts_with("commit ") {
            if !current_hash.is_empty() {
                commits.push(format!("{} {}", &current_hash[..current_hash.len().min(7)], current_subject.trim()));
            }
            current_hash = line.strip_prefix("commit ").unwrap_or("").trim().to_string();
            current_subject.clear();
        } else if line.starts_with("Author:") || line.starts_with("Date:") || line.starts_with("Merge:") {
            // Skip
        } else {
            let trimmed = line.trim();
            if !trimmed.is_empty() && current_subject.is_empty() {
                current_subject = trimmed.to_string();
            }
        }
    }
    if !current_hash.is_empty() {
        commits.push(format!("{} {}", &current_hash[..current_hash.len().min(7)], current_subject.trim()));
    }

    if commits.is_empty() {
        return output.to_string();
    }
    commits.join("\n")
}

fn format_git_diff(output: &str) -> String {
    let mut result = Vec::new();
    let mut context_count = 0;

    for line in output.lines() {
        if line.starts_with("diff --git") || line.starts_with("---") || line.starts_with("+++") {
            result.push(line.to_string());
            context_count = 0;
        } else if line.starts_with("@@") {
            result.push(line.to_string());
            context_count = 0;
        } else if line.starts_with('+') || line.starts_with('-') {
            result.push(line.to_string());
            context_count = 0;
        } else {
            context_count += 1;
            if context_count <= 1 {
                result.push(line.to_string());
            }
        }
    }
    result.join("\n")
}

fn format_git_short(subcmd: &str, output: &str) -> String {
    match subcmd {
        "add" => {
            if output.trim().is_empty() { return "ok".to_string(); }
            output.to_string()
        }
        "commit" => {
            for line in output.lines() {
                if line.contains(']') && line.contains('[') {
                    return format!("ok {}", line.trim());
                }
            }
            if output.trim().is_empty() { return "ok".to_string(); }
            output.lines().find(|l| !l.trim().is_empty()).unwrap_or("ok").to_string()
        }
        "push" => {
            for line in output.lines() {
                if line.contains("->") {
                    return format!("ok {}", line.trim());
                }
            }
            "ok".to_string()
        }
        "pull" => {
            let mut files_changed = 0;
            let mut insertions = 0;
            let mut deletions = 0;
            for line in output.lines() {
                if line.contains("files changed") || line.contains("file changed") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    for (i, p) in parts.iter().enumerate() {
                        if *p == "file" || p.starts_with("file") { files_changed = parts.get(i-1).and_then(|n| n.parse().ok()).unwrap_or(0); }
                        if p.starts_with("insertion") { insertions = parts.get(i-1).and_then(|n| n.parse().ok()).unwrap_or(0); }
                        if p.starts_with("deletion") { deletions = parts.get(i-1).and_then(|n| n.parse().ok()).unwrap_or(0); }
                    }
                }
            }
            if files_changed > 0 {
                format!("ok {} files +{} -{}", files_changed, insertions, deletions)
            } else if output.contains("Already up to date") {
                "ok up-to-date".to_string()
            } else {
                "ok".to_string()
            }
        }
        _ => output.lines().take(3).collect::<Vec<_>>().join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_status_clean() {
        let output = "On branch main\nnothing to commit, working tree clean\n";
        assert_eq!(format_git_status(output), "clean");
    }

    #[test]
    fn test_git_log_compact() {
        let output = "commit abc1234567890\nAuthor: Test <test@test.com>\nDate:   Mon Apr 13\n\n    feat: Add feature\n\ncommit def5678901234\nAuthor: Test <test@test.com>\nDate:   Sun Apr 12\n\n    fix: Bug fix\n";
        let result = format_git_log(output);
        assert!(result.contains("abc1234"));
        assert!(result.contains("feat: Add feature"));
        assert!(!result.contains("Author:"));
    }

    #[test]
    fn test_git_push_compact() {
        let output = "Enumerating objects: 5, done.\nCounting objects: 100% (5/5), done.\nDelta compression using up to 8 threads\n   abc1234..def5678  main -> main\n";
        let result = format_git_short("push", output);
        assert!(result.starts_with("ok"));
    }
}
