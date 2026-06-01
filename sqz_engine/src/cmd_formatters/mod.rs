mod preprocess;
pub mod truncate;
pub mod test_output;
mod git;
mod cargo;
mod npm;
mod docker;
mod kubectl;
mod system;
mod js;
pub mod lint;

pub fn format_command(cmd: &str, output: &str) -> Option<String> {
    let cleaned = preprocess::clean(output);
    let input = if cleaned.len() < output.len() { &cleaned } else { output };

    dispatch(cmd, input)
}

fn dispatch(cmd: &str, output: &str) -> Option<String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let base = parts.first().map(|s| s.rsplit('/').next().unwrap_or(s)).unwrap_or("");

    match base {
        "git" => git::format_git(parts.get(1).copied(), output),
        "cargo" => cargo::format_cargo(parts.get(1).copied(), output),
        "npm" | "npx" => npm::format_npm(parts.get(1).copied(), output),
        "pnpm" => npm::format_pnpm(parts.get(1).copied(), output),
        "yarn" | "bun" => npm::format_npm(parts.get(1).copied(), output),
        "pytest" | "python" if cmd.contains("pytest") => Some(test_output::format_test_failures(output)),
        "go" if parts.get(1).copied() == Some("test") => Some(test_output::format_test_failures(output)),
        "docker" | "podman" => docker::format_docker(parts.get(1).copied(), output),
        "kubectl" => kubectl::format_kubectl(parts.get(1).copied(), output),
        "ls" => Some(system::format_ls(output)),
        "find" | "fd" => Some(system::format_find(output)),
        "tsc" => Some(js::format_tsc(output)),
        "eslint" | "biome" => Some(lint::format_lint(output)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_command_routing() {
        assert!(format_command("git status", "nothing to commit").is_some());
        assert!(format_command("cargo test", "test result: ok").is_some());
        assert!(format_command("unknown_tool", "output").is_none());
    }

    #[test]
    fn test_ansi_stripped_before_dispatch() {
        let output = "\x1b[32mOn branch main\x1b[0m\nnothing to commit, working tree clean\n";
        let result = format_command("git status", output);
        assert_eq!(result, Some("clean".to_string()));
    }
}
