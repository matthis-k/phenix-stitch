from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise RuntimeError(f"{label}: source pattern not found")
    return text.replace(old, new, 1)


path = Path("crates/stitch/src/workloop.rs")
text = path.read_text()

text = replace_once(
    text,
    '''
    /// Run `git` and return stdout as a trimmed string.
    fn run_git(&self, repo: &Path, args: &[&str]) -> Result<String, String> {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .map_err(|e| format!("failed to run git: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git {} failed: {}", args.join(" "), stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
''',
    "\n",
    "unused JjBackend::run_git",
)

text = replace_once(
    text,
    '''        let refs = PublishRefs {
            targets: vec![PublishTarget {
                name: "repo".to_string(),
                path: repo_path.clone(),
                bookmark: "main".to_string(),
            }],
            repos: vec![],
            main_bookmarks: vec![],
        };
''',
    '''        let refs: PublishRefs = serde_json::from_value(serde_json::json!({
            "targets": [{
                "name": "repo",
                "path": repo_path,
                "bookmark": "main",
            }]
        }))
        .unwrap();
''',
    "deprecated PublishRefs test fields",
)

path.write_text(text)
