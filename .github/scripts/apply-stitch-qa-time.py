import re
from pathlib import Path


def replace(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if text.count(old) != 1:
        raise SystemExit(f"{path}: expected exactly one {old!r}")
    file.write_text(text.replace(old, new, 1))


def regex(path: str, pattern: str, replacement: str) -> None:
    file = Path(path)
    text = file.read_text()
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise SystemExit(f"{path}: pattern did not match exactly once: {pattern}")
    file.write_text(updated)


replace("crates/stitch/Cargo.toml", 'serde_json = "1"\n', 'serde_json = "1"\nchrono = "0.4"\n')
replace("crates/stitch/src/lib.rs", "pub mod status;\n", "pub mod status;\npub(crate) mod time;\n")

Path("crates/stitch/src/time.rs").write_text('''use chrono::{SecondsFormat, Utc};

pub fn utc_date() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

pub fn utc_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_is_second_precision_rfc3339() {
        let value = utc_timestamp();
        assert_eq!(value.len(), 20);
        assert_eq!(&value[10..11], "T");
        assert!(value.ends_with('Z'));
    }
}
''')

replace("crates/stitch/src/model.rs", "    let today = chrono_now();\n", "    let today = crate::time::utc_date();\n")
regex("crates/stitch/src/model.rs", r"fn chrono_now\(\) -> String \{.*?\n\}\n\n(?=fn slugify)", "")
regex("crates/stitch/src/sync.rs", r"fn timestamp_now\(\) -> String \{.*?\n\}\n\n(?=fn default_message)", "")
replace("crates/stitch/src/sync.rs", "        started_at: timestamp_now(),\n", "        started_at: crate::time::utc_timestamp(),\n")
replace("crates/stitch/src/workloop.rs", "use std::time::SystemTime;\n", "")
regex(
    "crates/stitch/src/workloop.rs",
    r"/// Simple RFC-3339-like timestamp without external deps\.\n(#\[derive.*?\npub struct Timestamp\(String\);\n\n)impl Timestamp \{.*?\n\}\n\n(?=// -+\n// LoopBackend trait)",
    "/// RFC 3339 UTC timestamp.\n\\1impl Timestamp {\n    pub fn now() -> Self {\n        Self(crate::time::utc_timestamp())\n    }\n}\n\n",
)

validate = Path("crates/stitch/src/validate.rs")
text = validate.read_text()
text = text.replace(
    '        let dir = std::env::temp_dir().join("__stitch_test_git_repo__");\n        let _ = std::fs::remove_dir_all(&dir);\n        std::fs::create_dir_all(&dir).unwrap();\n',
    '        let dir = tempfile::tempdir().unwrap();\n        let repo_path = dir.path();\n',
    1,
)
text = text.replace('.current_dir(&dir)', '.current_dir(repo_path)', 3)
text = text.replace('        let path_str = dir.to_string_lossy().to_string();\n', '        let path_str = repo_path.to_string_lossy().to_string();\n', 1)
text = text.replace('        let _ = std::fs::remove_dir_all(&dir);\n', '', 1)
text = text.replace(
    '    fn test_validate_non_existent_repo() {\n        let cfg = WorkspaceConfig {\n',
    '    fn test_validate_non_existent_repo() {\n        let dir = tempfile::tempdir().unwrap();\n        let missing_path = dir.path().join("missing-repo").to_string_lossy().to_string();\n        let cfg = WorkspaceConfig {\n',
    1,
)
text = text.replace('path: "/tmp/__stitch_test_ghost__".to_string(),', 'path: missing_path.clone(),', 1)
text = text.replace('path: "/tmp/__stitch_test_ghost__".to_string(),', 'path: missing_path,', 1)
validate.write_text(text)
