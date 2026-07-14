from __future__ import annotations

from pathlib import Path

SYNC = Path("crates/stitch/src/sync.rs")
text = SYNC.read_text()


def replace_once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one occurrence, found {count}: {old[:120]!r}")
    text = text.replace(old, new, 1)


replace_once(
    '''fn action_id(action: &Action) -> String {
    match action {
        Action::Commit { node, .. } => format!("commit-{}", node),
        Action::UpdateInputs { node, .. } => format!("update-{}", node),
        Action::Validate { node } => format!("validate-{}", node),
        Action::Push { node } => format!("push-{}", node),
    }
}
''',
    '''fn action_id(action: &Action) -> String {
    match action {
        Action::Commit { node, .. } => format!("commit-{}", node),
        Action::UpdateInputs { node, .. } => format!("update-{}", node),
        Action::Validate { node } => format!("validate-{}", node),
        Action::Push { node } => format!("push-{}", node),
    }
}

const ACTION_TRAILER: &str = "Stitch-Action";

fn action_commit_message(
    message: &str,
    transaction_id: &str,
    workspace: &str,
    action: &Action,
) -> String {
    let mut message = crate::model::add_trailers(message, transaction_id, workspace);
    message.push_str(&format!("{}: {}\\n", ACTION_TRAILER, action_id(action)));
    message
}

fn committed_action_head(
    repo: &Path,
    transaction_id: &str,
    action: &Action,
) -> Result<Option<String>, String> {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%B", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|error| format!("Inspect committed action: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Inspect committed action in '{}': {}",
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let message = String::from_utf8_lossy(&output.stdout);
    let transaction_trailer = format!("Change-Set: {transaction_id}");
    let action_trailer = format!("{}: {}", ACTION_TRAILER, action_id(action));
    let has_transaction = message.lines().any(|line| line.trim() == transaction_trailer);
    let has_action = message.lines().any(|line| line.trim() == action_trailer);

    if has_transaction && has_action {
        return git::git_head(repo).map(Some);
    }

    Ok(None)
}

fn remote_branch_head(repo: &Path, branch: &str) -> Result<Option<String>, String> {
    let reference = format!("refs/heads/{branch}");
    let output = std::process::Command::new("git")
        .args(["ls-remote", "--heads", "origin", &reference])
        .current_dir(repo)
        .output()
        .map_err(|error| format!("Inspect remote branch '{branch}': {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Inspect remote branch '{}' in '{}': {}",
            branch,
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn branch_is_published(repo: &Path, branch: &str, expected_sha: &str) -> Result<bool, String> {
    Ok(remote_branch_head(repo, branch)?.as_deref() == Some(expected_sha))
}
''',
)

replace_once(
    '''            git::git_add(&node.path, &files)?;
            let trailed = crate::model::add_trailers(&msg, &plan.transaction_id, &cfg.workspace);
            git::git_commit(&node.path, &trailed)?;
''',
    '''            git::git_add(&node.path, &files)?;
            let trailed = action_commit_message(&msg, &plan.transaction_id, &cfg.workspace, action);
            git::git_commit(&node.path, &trailed)?;
''',
)

replace_once(
    '''                let trailed =
                    crate::model::add_trailers(&msg, &plan.transaction_id, &cfg.workspace);
                git::git_commit(&node.path, &trailed)?;
''',
    '''                let trailed =
                    action_commit_message(&msg, &plan.transaction_id, &cfg.workspace, action);
                git::git_commit(&node.path, &trailed)?;
''',
)

replace_once(
    '''            let result = git_push(&node.path, branch);
            match result {
                Ok(()) => {
                    push_results.insert(node.name.clone(), Ok(()));
''',
    '''            let local_head = git::git_head(&node.path)?;
            if branch_is_published(&node.path, branch, &local_head).unwrap_or(false) {
                push_results.insert(node.name.clone(), Ok(()));
                if let Some(entry) = journal.nodes.get_mut(node_id) {
                    entry.pushed = true;
                }
                let action_id = action_id(action);
                if let Some(entry) = journal
                    .actions
                    .iter_mut()
                    .find(|entry| entry.action_id == action_id)
                {
                    entry.pushed = true;
                }
                return Ok(());
            }

            let result = git_push(&node.path, branch);
            match result {
                Ok(()) => {
                    if !branch_is_published(&node.path, branch, &local_head)? {
                        return Err(format!(
                            "Push for '{}' returned success but origin/{} does not resolve to {}",
                            node.name, branch, local_head
                        ));
                    }
                    push_results.insert(node.name.clone(), Ok(()));
''',
)

replace_once(
    '''    // Resume from journal.actions, not a newly computed plan
    for entry in &resume_journal.actions.clone() {
        if entry.state == ActionState::Done {
            if matches!(entry.action, Action::Push { .. }) && entry.pushed {
                push_results.insert(entry.node.clone(), Ok(()));
            }
            continue;
        }

        match &entry.action {
''',
    '''    // Resume from journal.actions, not a newly computed plan.
    // A process can terminate after Git completed an operation but before the
    // journal was persisted. Reconcile durable Git state before replaying it.
    for entry in &resume_journal.actions.clone() {
        if entry.state == ActionState::Done {
            if matches!(entry.action, Action::Push { .. }) && entry.pushed {
                push_results.insert(entry.node.clone(), Ok(()));
            }
            continue;
        }

        if matches!(entry.action, Action::Commit { .. } | Action::UpdateInputs { .. }) {
            let node_path = resume_journal
                .nodes
                .get(&entry.node)
                .map(|node| Path::new(&node.path))
                .unwrap_or(Path::new("."));
            if let Some(sha) = committed_action_head(node_path, transaction_id, &entry.action)? {
                commit_shas.insert(entry.node.clone(), sha.clone());
                if let Some(action_entry) = resume_journal
                    .actions
                    .iter_mut()
                    .find(|candidate| candidate.action_id == entry.action_id)
                {
                    action_entry.state = ActionState::Done;
                    action_entry.commit_sha = Some(sha.clone());
                    action_entry.error = None;
                }
                if let Some(node_entry) = resume_journal.nodes.get_mut(&entry.node) {
                    node_entry.commit_sha = Some(sha);
                }
                write_journal(&resume_journal, cfg)?;
                continue;
            }
        }

        match &entry.action {
''',
)

replace_once(
    '''                if current_files.is_empty() {
                    if let Some(e) = resume_journal
                        .actions
                        .iter_mut()
                        .find(|e| e.action_id == entry.action_id)
                    {
                        e.state = ActionState::Done;
                    }
                    write_journal(&resume_journal, cfg)?;
                    continue;
                }
''',
    '''                if current_files.is_empty() {
                    return Err(format!(
                        "Resume refused for '{}': no pending files and HEAD has no matching {} trailer for action '{}'. The transaction cannot prove that the commit completed.",
                        node_id, ACTION_TRAILER, entry.action_id
                    ));
                }
''',
)

replace_once(
    '''                git::git_add(Path::new(&node_path), &current_files)?;
                let trailed = crate::model::add_trailers(&msg, transaction_id, &cfg.workspace);
                git::git_commit(Path::new(&node_path), &trailed)?;
''',
    '''                git::git_add(Path::new(&node_path), &current_files)?;
                let trailed =
                    action_commit_message(&msg, transaction_id, &cfg.workspace, &entry.action);
                git::git_commit(Path::new(&node_path), &trailed)?;
''',
)

replace_once(
    '''                    let msg = format!("chore(inputs): resume sync for {}", node_id);
                    let trailed = crate::model::add_trailers(&msg, transaction_id, &cfg.workspace);
                    git::git_commit(Path::new(&node_path), &trailed)?;
''',
    '''                    let msg = format!("chore(inputs): resume sync for {}", node_id);
                    let trailed =
                        action_commit_message(&msg, transaction_id, &cfg.workspace, &entry.action);
                    git::git_commit(Path::new(&node_path), &trailed)?;
''',
)

replace_once(
    '''                let result = git_push(Path::new(&node_path), branch);
                if let Err(ref e) = result {
''',
    '''                let local_head = git::git_head(Path::new(&node_path))?;
                if branch_is_published(Path::new(&node_path), branch, &local_head)
                    .unwrap_or(false)
                {
                    push_results.insert(node_id.clone(), Ok(()));
                    if let Some(action_entry) = resume_journal
                        .actions
                        .iter_mut()
                        .find(|candidate| candidate.action_id == entry.action_id)
                    {
                        action_entry.state = ActionState::Done;
                        action_entry.pushed = true;
                        action_entry.error = None;
                    }
                    if let Some(node_entry) = resume_journal.nodes.get_mut(node_id) {
                        node_entry.pushed = true;
                    }
                    write_journal(&resume_journal, cfg)?;
                    continue;
                }

                let result = git_push(Path::new(&node_path), branch);
                if let Err(ref e) = result {
''',
)

replace_once(
    '''                push_results.insert(node_id.clone(), Ok(()));
                if let Some(je) = resume_journal
''',
    '''                if !branch_is_published(Path::new(&node_path), branch, &local_head)? {
                    return Err(format!(
                        "Push for '{}' returned success but origin/{} does not resolve to {}",
                        node_id, branch, local_head
                    ));
                }

                push_results.insert(node_id.clone(), Ok(()));
                if let Some(je) = resume_journal
''',
)

replace_once(
    '''fn write_journal(journal: &TransactionJournal, cfg: &WorkspaceConfig) -> Result<(), String> {
    let dir = journal_dir(cfg)?;
    let path = dir.join(format!("{}.json", journal.transaction_id));
    let content =
        serde_json::to_string_pretty(journal).map_err(|e| format!("Serialize journal: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("Write journal: {}", e))?;
    Ok(())
}
''',
    '''fn write_journal(journal: &TransactionJournal, cfg: &WorkspaceConfig) -> Result<(), String> {
    use std::io::Write as _;

    let dir = journal_dir(cfg)?;
    let path = dir.join(format!("{}.json", journal.transaction_id));
    let temp_path = dir.join(format!(
        ".{}.{}.tmp",
        journal.transaction_id,
        std::process::id()
    ));
    let content =
        serde_json::to_vec_pretty(journal).map_err(|e| format!("Serialize journal: {}", e))?;

    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp_path)
            .map_err(|e| format!("Create temporary journal: {}", e))?;
        file.write_all(&content)
            .map_err(|e| format!("Write temporary journal: {}", e))?;
        file.sync_all()
            .map_err(|e| format!("Sync temporary journal: {}", e))?;
        std::fs::rename(&temp_path, &path).map_err(|e| format!("Replace journal: {}", e))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}
''',
)

replace_once(
    '''        use std::io::Write;
        let dir = std::env::temp_dir().join("__sync_test_flake_lock");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
''',
    '''        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
''',
)

replace_once(
    '''        let mut f = std::fs::File::create(dir.join("flake.lock")).unwrap();
''',
    '''        let mut f = std::fs::File::create(dir.path().join("flake.lock")).unwrap();
''',
)

replace_once(
    '''            &dir,
            "phenix-pins",
''',
    '''            dir.path(),
            "phenix-pins",
''',
)

replace_once(
    '''
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_input_sync_message() {
''',
    '''
    }

    #[test]
    fn test_input_sync_message() {
''',
)

replace_once(
    '''    #[test]
    fn test_generate_transaction_id() {
        let id = generate_transaction_id();
        assert!(id.starts_with("sync-"));
        assert!(id.len() > 10);
    }
}
''',
    '''    #[test]
    fn test_generate_transaction_id() {
        let id = generate_transaction_id();
        assert!(id.starts_with("sync-"));
        assert!(id.len() > 10);
    }

    #[test]
    fn action_commit_message_carries_replay_identity() {
        let action = Action::Commit {
            node: "phenix-stitch".to_string(),
            message: "ignored".to_string(),
        };
        let message = action_commit_message("fix: replay safety", "tx-123", "phenix", &action);

        assert!(message.contains("Change-Set: tx-123"));
        assert!(message.contains("Stitch-Action: commit-phenix-stitch"));
    }

    #[test]
    fn committed_action_head_recovers_the_exact_action() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "stitch@example.invalid"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Stitch Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let action = Action::Commit {
            node: "repo".to_string(),
            message: "ignored".to_string(),
        };
        let message = action_commit_message("fix: replay safety", "tx-123", "phenix", &action);
        git::git_commit(dir.path(), &message).unwrap();

        let recovered = committed_action_head(dir.path(), "tx-123", &action)
            .unwrap()
            .expect("matching action should be recoverable");
        assert_eq!(recovered, git::git_head(dir.path()).unwrap());

        let other = Action::UpdateInputs {
            node: "repo".to_string(),
            updates: Vec::new(),
            message: "ignored".to_string(),
        };
        assert!(committed_action_head(dir.path(), "tx-123", &other)
            .unwrap()
            .is_none());
    }
}
''',
)

SYNC.write_text(text)
