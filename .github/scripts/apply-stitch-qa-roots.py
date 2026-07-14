from pathlib import Path

path = Path("crates/phenix-mcp-core/src/mcp.rs")
text = path.read_text()

old = '''fn extract_rootable_paths(input: &Value) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for key in &["root", "path", "dir", "directory"] {
        if let Some(v) = input.get(*key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                paths.push(std::path::PathBuf::from(v));
            }
        }
    }
    paths
}
'''
new = '''fn extract_rootable_paths(input: &Value) -> Vec<std::path::PathBuf> {
    const PATH_KEYS: &[&str] = &[
        "root",
        "path",
        "repo",
        "cwd",
        "dir",
        "directory",
        "workspace_root",
    ];

    let mut paths = Vec::new();
    for key in PATH_KEYS {
        if let Some(value) = input.get(*key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                paths.push(std::path::PathBuf::from(value));
            }
        }
    }
    paths
}
'''
if text.count(old) != 1:
    raise SystemExit(f"expected one root-path extractor, found {text.count(old)}")
text = text.replace(old, new, 1)

text += '''

#[cfg(test)]
mod tests {
    use super::extract_rootable_paths;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn repo_argument_is_subject_to_root_validation() {
        let paths = extract_rootable_paths(&json!({
            "repo": "../outside",
            "unrelated": "ignored"
        }));
        assert_eq!(paths, vec![PathBuf::from("../outside")]);
    }

    #[test]
    fn all_supported_path_fields_are_collected() {
        let paths = extract_rootable_paths(&json!({
            "root": "root",
            "path": "path",
            "repo": "repo",
            "cwd": "cwd",
            "dir": "dir",
            "directory": "directory",
            "workspace_root": "workspace",
            "empty": ""
        }));
        assert_eq!(paths.len(), 7);
    }
}
'''

path.write_text(text)
