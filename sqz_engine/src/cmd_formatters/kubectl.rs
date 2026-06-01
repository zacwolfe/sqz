pub fn format_kubectl(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "get" => Some(format_kubectl_get(output)),
        _ => None,
    }
}

fn format_kubectl_get(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() { return output.to_string(); }

    let mut result = Vec::new();
    for line in &lines {
        let collapsed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        result.push(collapsed);
    }
    result.join("\n")
}
