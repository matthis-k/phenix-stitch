use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git;
use crate::graph::{NodeId, WorkspaceGraph};
#[allow(unused_imports)]
use crate::model::{RepoAvailability, RepoStatus, WorkspaceConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    pub transaction_id: String,
    pub root: NodeId,
    pub affected_nodes: BTreeSet<NodeId>,
    pub actions: Vec<Action>,
    pub node_plans: BTreeMap<NodeId, NodeCommitPlan>,
    pub blocked_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Commit {
        node: NodeId,
        message: String,
    },
    UpdateInputs {
        node: NodeId,
        updates: Vec<InputUpdate>,
        message: String,
    },
    Validate {
        node: NodeId,
    },
    Push {
        node: NodeId,
    },
}

impl Action {
    pub fn node(&self) -> &NodeId {
        match self {
            Self::Commit { node, .. } => node,
            Self::UpdateInputs { node, .. } => node,
            Self::Validate { node } => node,
            Self::Push { node } => node,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCommitPlan {
    pub node: NodeId,
    pub dirty: bool,
    pub needs_code_commit: bool,
    pub needs_input_sync: bool,
    pub dependencies_to_update: Vec<InputUpdate>,
    pub message: String,
    pub validation_commands: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputUpdate {
    pub input_name: String,
    pub dependency_node: NodeId,
    pub target_rev: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionJournal {
    pub transaction_id: String,
    pub started_at: String,
    pub root: NodeId,
    pub phase: JournalPhase,
    pub nodes: BTreeMap<NodeId, NodeJournalEntry>,
    #[serde(default)]
    pub actions: Vec<ActionJournalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JournalPhase {
    Planned,
    Committed,
    Validated,
    Pushing,
    Completed,
    Failed,
}

impl std::fmt::Display for JournalPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JournalPhase::Planned => write!(f, "planned"),
            JournalPhase::Committed => write!(f, "committed"),
            JournalPhase::Validated => write!(f, "validated"),
            JournalPhase::Pushing => write!(f, "pushing"),
            JournalPhase::Completed => write!(f, "completed"),
            JournalPhase::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeJournalEntry {
    pub path: String,
    pub commit_sha: Option<String>,
    pub pushed: bool,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ActionState {
    Pending,
    Running,
    Done,
    Failed,
}

fn action_id(action: &Action) -> String {
    match action {
        Action::Commit { node, .. } => format!("commit-{}", node),
        Action::UpdateInputs { node, .. } => format!("update-{}", node),
        Action::Validate { node } => format!("validate-{}", node),
        Action::Push { node } => format!("push-{}", node),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionJournalEntry {
    pub action_id: String,
    pub node: NodeId,
    pub action: Action,
    pub state: ActionState,
    pub commit_sha: Option<String>,
    pub pushed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn generate_transaction_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rand_suffix: u32 = rand_byte();
    format!("sync-{:x}-{:04x}", secs, rand_suffix)
}

fn rand_byte() -> u32 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    t & 0xFFFF
}

fn timestamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    let y = 1970_f64 + (days as f64 - 1.0) / 365.25;
    let year = y as u64;
    let remaining = days - ((year - 1970) * 365 + (year - 1969) / 4);
    let month_days = [
        31,
        if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    let mut day = remaining;
    for &md in &month_days {
        if day <= md {
            break;
        }
        day -= md;
        month += 1;
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day + 1,
        hours,
        minutes,
        seconds
    )
}

fn default_message(node_name: &str, sha: Option<&str>) -> String {
    match sha {
        Some(s) => format!(
            "chore(stitch): sync DAG inputs\n\nIncludes updates from:\n- {}: {}",
            node_name, s
        ),
        None => "chore(stitch): commit workspace changes".to_string(),
    }
}

fn input_sync_message(node_name: &str, updates: &[InputUpdate]) -> String {
    let mut msg = format!("chore(inputs): sync {} inputs", node_name);
    for u in updates {
        if let Some(rev) = &u.target_rev {
            msg.push_str(&format!("\n- {}: {}", u.dependency_node, rev));
        }
    }
    msg
}

pub fn plan_sync(
    graph: &WorkspaceGraph,
    statuses: &[RepoStatus],
    _cfg: &WorkspaceConfig,
) -> Result<ActionPlan, String> {
    plan_commit(graph, statuses, _cfg, true)
}

pub fn plan_commit(
    graph: &WorkspaceGraph,
    statuses: &[RepoStatus],
    _cfg: &WorkspaceConfig,
    include_push: bool,
) -> Result<ActionPlan, String> {
    let mut blocked_reasons = Vec::new();
    let transaction_id = generate_transaction_id();
    let topo_order = graph.topological_order()?;

    let dirty_nodes: BTreeSet<NodeId> = statuses
        .iter()
        .filter(|s| s.staged_count > 0 || s.unstaged_count > 0 || s.untracked_count > 0)
        .map(|s| s.name.clone())
        .collect();

    let mut affected_nodes: BTreeSet<NodeId> = dirty_nodes.clone();
    let mut plan_nodes: BTreeMap<NodeId, NodeCommitPlan> = BTreeMap::new();

    for node_id in &topo_order {
        let is_dirty = dirty_nodes.contains(node_id);

        let mut deps_to_update = Vec::new();
        for edge in graph.dependencies_of(node_id) {
            if affected_nodes.contains(&edge.to) {
                deps_to_update.push(InputUpdate {
                    input_name: edge.input_name.clone(),
                    dependency_node: edge.to.clone(),
                    target_rev: None,
                });
            }
        }

        let needs_code_commit = is_dirty;
        let needs_input_sync = !deps_to_update.is_empty();

        if needs_code_commit || needs_input_sync {
            affected_nodes.insert(node_id.clone());
        }

        let message = if needs_code_commit {
            default_message(node_id, None)
        } else if needs_input_sync {
            let short_updates: Vec<InputUpdate> = deps_to_update
                .iter()
                .map(|u| InputUpdate {
                    input_name: u.input_name.clone(),
                    dependency_node: u.dependency_node.clone(),
                    target_rev: u.target_rev.clone(),
                })
                .collect();
            input_sync_message(node_id, &short_updates)
        } else {
            String::new()
        };

        let validation_commands: Vec<Vec<String>> = Vec::new();

        plan_nodes.insert(
            node_id.clone(),
            NodeCommitPlan {
                node: node_id.clone(),
                dirty: is_dirty,
                needs_code_commit,
                needs_input_sync,
                dependencies_to_update: deps_to_update,
                message,
                validation_commands,
            },
        );
    }

    for node_id in &topo_order {
        let node = match graph.get_node(node_id) {
            Some(n) => n,
            None => continue,
        };
        let plan = match plan_nodes.get(node_id) {
            Some(p) => p,
            None => continue,
        };
        if !plan.needs_code_commit && !plan.needs_input_sync {
            continue;
        }

        if node.branch == "HEAD" {
            blocked_reasons.push(format!("{}: detached HEAD (use --force)", node.name));
        }
    }

    let node_plans: BTreeMap<NodeId, NodeCommitPlan> = plan_nodes
        .into_iter()
        .filter(|(id, _)| affected_nodes.contains(id))
        .collect();

    // Build flat action list in DAG order:
    //   1. Commit + UpdateInputs for each affected node (topo order)
    //   2. Validate each affected node
    //   3. Push each affected node
    let mut actions: Vec<Action> = Vec::new();
    for node_id in topo_order.iter().filter(|id| affected_nodes.contains(*id)) {
        let plan = match node_plans.get(node_id) {
            Some(p) => p,
            None => continue,
        };
        if plan.needs_code_commit {
            actions.push(Action::Commit {
                node: node_id.clone(),
                message: default_message(node_id, None),
            });
        }
        if plan.needs_input_sync {
            let input_msg = input_sync_message(node_id, &plan.dependencies_to_update);
            actions.push(Action::UpdateInputs {
                node: node_id.clone(),
                updates: plan.dependencies_to_update.clone(),
                message: input_msg,
            });
        }
    }
    for node_id in topo_order.iter().filter(|id| affected_nodes.contains(*id)) {
        actions.push(Action::Validate {
            node: node_id.clone(),
        });
    }
    if include_push {
        for node_id in topo_order.iter().filter(|id| affected_nodes.contains(*id)) {
            actions.push(Action::Push {
                node: node_id.clone(),
            });
        }
    }

    Ok(ActionPlan {
        transaction_id,
        root: graph.root.clone(),
        affected_nodes,
        actions,
        node_plans,
        blocked_reasons,
    })
}

pub struct ActionResult {
    pub transaction_id: String,
    pub created_commits: BTreeMap<String, String>,
    pub push_results: BTreeMap<String, Result<(), String>>,
    pub phase: JournalPhase,
}

pub fn execute_sync(
    plan: &ActionPlan,
    graph: &WorkspaceGraph,
    cfg: &WorkspaceConfig,
    no_push: bool,
    messages: Option<&BTreeMap<String, String>>,
    force: bool,
) -> Result<ActionResult, String> {
    execute_plan(plan, graph, cfg, no_push, messages, force)
}

pub fn plan_local_commit(
    graph: &WorkspaceGraph,
    statuses: &[RepoStatus],
    cfg: &WorkspaceConfig,
) -> Result<ActionPlan, String> {
    let mut plan = plan_commit(graph, statuses, cfg, false)?;

    plan.actions.retain(|a| matches!(a, Action::Commit { .. }));

    for node_plan in plan.node_plans.values_mut() {
        node_plan.needs_input_sync = false;
        node_plan.dependencies_to_update.clear();
        node_plan.validation_commands.clear();
    }

    plan.affected_nodes = plan.actions.iter().map(|a| a.node().clone()).collect();

    Ok(plan)
}

pub fn execute_local_commit_plan(
    plan: &ActionPlan,
    graph: &WorkspaceGraph,
    cfg: &WorkspaceConfig,
    messages: Option<&BTreeMap<String, String>>,
    force: bool,
) -> Result<ActionResult, String> {
    execute_plan(plan, graph, cfg, true, messages, force)
}

fn execute_plan(
    plan: &ActionPlan,
    graph: &WorkspaceGraph,
    cfg: &WorkspaceConfig,
    no_push: bool,
    messages: Option<&BTreeMap<String, String>>,
    force: bool,
) -> Result<ActionResult, String> {
    let mut journal = TransactionJournal {
        transaction_id: plan.transaction_id.clone(),
        started_at: timestamp_now(),
        root: plan.root.clone(),
        phase: JournalPhase::Planned,
        nodes: BTreeMap::new(),
        actions: Vec::new(),
    };

    for action in plan.actions.iter() {
        let node_id = action.node();
        if !journal.nodes.contains_key(node_id) {
            let node = graph
                .get_node(node_id)
                .ok_or_else(|| format!("Node '{}' not found", node_id))?;
            journal.nodes.insert(
                node_id.clone(),
                NodeJournalEntry {
                    path: node.path.to_string_lossy().to_string(),
                    commit_sha: None,
                    pushed: false,
                    branch: Some(node.branch.clone()),
                },
            );
        }
        let expected_files = if matches!(action, Action::Commit { .. }) {
            let repo_path = journal
                .nodes
                .get(node_id)
                .map(|n| n.path.clone())
                .unwrap_or_default();
            collect_all_changed_files(Path::new(&repo_path)).unwrap_or_default()
        } else {
            Vec::new()
        };
        journal.actions.push(ActionJournalEntry {
            action_id: action_id(action),
            node: node_id.clone(),
            action: action.clone(),
            state: ActionState::Pending,
            commit_sha: None,
            pushed: false,
            expected_files,
            error: None,
        });
    }

    write_journal(&journal, cfg)?;

    if !force {
        for reason in &plan.blocked_reasons {
            if reason.contains("detached HEAD") {
                return Err(format!("Blocked: {}. Use --force to proceed.", reason));
            }
        }
    }

    let mut commit_shas: BTreeMap<NodeId, String> = BTreeMap::new();
    let mut push_results: BTreeMap<String, Result<(), String>> = BTreeMap::new();

    for (action_idx, action) in plan.actions.iter().enumerate() {
        // Mark Running before execution
        if let Some(entry) = journal.actions.get_mut(action_idx) {
            entry.state = ActionState::Running;
        }
        write_journal(&journal, cfg)?;

        let action_result = execute_single_action(
            action,
            action_idx,
            plan,
            graph,
            cfg,
            no_push,
            messages,
            &mut commit_shas,
            &mut push_results,
            &mut journal,
        );

        match action_result {
            Ok(()) => {
                if let Some(entry) = journal.actions.get_mut(action_idx) {
                    entry.state = ActionState::Done;
                }
                write_journal(&journal, cfg)?;
            }
            Err(err) => {
                if let Some(entry) = journal.actions.get_mut(action_idx) {
                    entry.state = ActionState::Failed;
                    entry.error = Some(err.clone());
                }
                journal.phase = JournalPhase::Failed;
                write_journal(&journal, cfg)?;
                return Err(err);
            }
        }
    }

    journal.phase = JournalPhase::Completed;
    write_journal(&journal, cfg)?;

    Ok(ActionResult {
        transaction_id: plan.transaction_id.clone(),
        created_commits: commit_shas,
        push_results,
        phase: JournalPhase::Completed,
    })
}

#[allow(clippy::too_many_arguments)]
fn execute_single_action(
    action: &Action,
    _action_idx: usize,
    plan: &ActionPlan,
    graph: &WorkspaceGraph,
    cfg: &WorkspaceConfig,
    no_push: bool,
    messages: Option<&BTreeMap<String, String>>,
    commit_shas: &mut BTreeMap<NodeId, String>,
    push_results: &mut BTreeMap<String, Result<(), String>>,
    journal: &mut TransactionJournal,
) -> Result<(), String> {
    match action {
        Action::Commit {
            node: node_id,
            message,
        } => {
            let node = match graph.get_node(node_id) {
                Some(n) => n,
                None => return Ok(()),
            };

            let files = collect_all_changed_files(&node.path)?;
            if files.is_empty() {
                return Err(format!("{} marked dirty but no changes found", node.name));
            }

            let msg = messages
                .and_then(|m| m.get(node_id))
                .cloned()
                .unwrap_or_else(|| message.clone());

            git::git_add(&node.path, &files)?;
            let trailed = crate::model::add_trailers(&msg, &plan.transaction_id, &cfg.workspace);
            git::git_commit(&node.path, &trailed)?;
            let sha = git::git_head(&node.path)?;
            commit_shas.insert(node_id.clone(), sha.clone());

            if let Some(entry) = journal.nodes.get_mut(node_id) {
                entry.commit_sha = Some(sha);
            }
            Ok(())
        }
        Action::UpdateInputs {
            node: node_id,
            updates,
            message,
        } => {
            let node = match graph.get_node(node_id) {
                Some(n) => n,
                None => return Ok(()),
            };

            let updated_deps: Vec<InputUpdate> = updates
                .iter()
                .map(|u| {
                    let rev = commit_shas.get(&u.dependency_node).cloned();
                    InputUpdate {
                        input_name: u.input_name.clone(),
                        dependency_node: u.dependency_node.clone(),
                        target_rev: rev,
                    }
                })
                .collect();

            if updated_deps.is_empty() {
                return Ok(());
            }

            for update in &updated_deps {
                let target_rev = update.target_rev.as_deref().unwrap_or("HEAD");
                update_flake_lock_input(&node.path, &update.input_name, target_rev)?;
            }

            let lock_path = node.path.join("flake.lock");
            if lock_path.exists() {
                let msg = messages
                    .and_then(|m| m.get(node_id))
                    .cloned()
                    .unwrap_or_else(|| message.clone());

                git::git_add(&node.path, &[lock_path.to_string_lossy().to_string()])?;
                let trailed =
                    crate::model::add_trailers(&msg, &plan.transaction_id, &cfg.workspace);
                git::git_commit(&node.path, &trailed)?;
                let sha = git::git_head(&node.path)?;
                commit_shas.insert(node_id.clone(), sha.clone());

                if let Some(entry) = journal.nodes.get_mut(node_id) {
                    entry.commit_sha = Some(sha);
                }
            }
            Ok(())
        }
        Action::Validate { node: node_id } => {
            let plan_node = match plan.node_plans.get(node_id) {
                Some(p) => p,
                None => return Ok(()),
            };
            let node = match graph.get_node(node_id) {
                Some(n) => n,
                None => return Ok(()),
            };

            let repo = match git::GitRepo::open(&node.path) {
                Ok(r) => r,
                Err(_) => return Ok(()),
            };
            let status = repo.status()?;
            if status.staged_count() > 0 {
                return Err(format!(
                    "Repo '{}' has uncommitted tracked changes after sync commit",
                    node.name
                ));
            }

            let lock_path = node.path.join("flake.lock");
            if lock_path.exists() && plan_node.needs_input_sync {
                for update in &plan_node.dependencies_to_update {
                    let expected_rev = commit_shas.get(&update.dependency_node);
                    verify_lockfile_rev(
                        &lock_path,
                        &update.input_name,
                        expected_rev.map(|s| s.as_str()),
                    )?;
                }
            }

            for cmd_parts in &plan_node.validation_commands {
                if cmd_parts.is_empty() {
                    continue;
                }
                let output = std::process::Command::new(&cmd_parts[0])
                    .args(&cmd_parts[1..])
                    .current_dir(&node.path)
                    .output()
                    .map_err(|e| format!("Validation command failed: {}", e))?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!(
                        "Validation failed in '{}': {} {}",
                        node.name,
                        cmd_parts.join(" "),
                        stderr.trim()
                    ));
                }
            }
            Ok(())
        }
        Action::Push { node: node_id } => {
            if no_push {
                return Ok(());
            }

            let node = match graph.get_node(node_id) {
                Some(n) => n,
                None => return Ok(()),
            };

            let branch = journal
                .nodes
                .get(node_id)
                .and_then(|n| n.branch.as_deref())
                .unwrap_or(&node.branch);

            let result = git_push(&node.path, branch);
            match result {
                Ok(()) => {
                    push_results.insert(node.name.clone(), Ok(()));
                    if let Some(entry) = journal.nodes.get_mut(node_id) {
                        entry.pushed = true;
                    }
                    let action_id = action_id(action);
                    if let Some(entry) = journal
                        .actions
                        .iter_mut()
                        .find(|e| e.action_id == action_id)
                    {
                        entry.pushed = true;
                    }
                    Ok(())
                }
                Err(e) => {
                    push_results.insert(node.name.clone(), Err(e.clone()));
                    let pushed_nodes: Vec<String> = push_results
                        .iter()
                        .filter(|(_, r)| r.is_ok())
                        .map(|(name, _)| name.clone())
                        .collect();
                    let failed_nodes: Vec<String> = push_results
                        .iter()
                        .filter(|(_, r)| r.is_err())
                        .map(|(name, _)| name.clone())
                        .collect();
                    Err(format!(
                        "Push failed for '{}': {}\nPushed: {}\nFailed: {}\nResume: stitch commit --resume {}",
                        node.name, e, pushed_nodes.join(", "), failed_nodes.join(", "), plan.transaction_id
                    ))
                }
            }
        }
    }
}

fn collect_all_changed_files(repo: &Path) -> Result<Vec<String>, String> {
    let git_repo = git::GitRepo::open(repo)?;
    let status = git_repo.status()?;
    let mut files: Vec<String> = status
        .all_files()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    files.sort();
    Ok(files)
}

fn update_flake_lock_input(repo_path: &Path, input_name: &str, rev: &str) -> Result<(), String> {
    let lock_path = repo_path.join("flake.lock");
    if !lock_path.exists() {
        return Ok(());
    }

    let output = std::process::Command::new("nix")
        .args(["flake", "update", input_name])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("nix flake update {}: {}", input_name, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "nix flake update {} failed (transaction is resumable): {}",
            input_name,
            stderr.trim()
        ));
    }

    let new_content = std::fs::read_to_string(&lock_path)
        .map_err(|e| format!("Read updated flake.lock: {}", e))?;
    let new_lock: serde_json::Value = serde_json::from_str(&new_content)
        .map_err(|e| format!("Parse updated flake.lock: {}", e))?;

    let new_rev = new_lock
        .get("nodes")
        .and_then(|n| n.get(input_name))
        .and_then(|n| n.get("locked"))
        .and_then(|l| l.get("rev"))
        .and_then(|r| r.as_str());

    match new_rev {
        Some(r) if r == rev => Ok(()),
        Some(r) => Err(format!(
            "nix flake update for '{}' resolved to {} but expected {} (transaction is resumable)",
            input_name, r, rev
        )),
        None => Err(format!(
            "nix flake update for '{}' did not set a 'rev' field (transaction is resumable)",
            input_name
        )),
    }
}

fn verify_lockfile_rev(
    lock_path: &Path,
    input_name: &str,
    expected_rev: Option<&str>,
) -> Result<(), String> {
    if !lock_path.exists() {
        return Ok(());
    }

    let content =
        std::fs::read_to_string(lock_path).map_err(|e| format!("Read flake.lock: {}", e))?;
    let lock: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Parse flake.lock: {}", e))?;

    let rev = lock
        .get("nodes")
        .and_then(|n| n.get(input_name))
        .and_then(|n| n.get("locked"))
        .and_then(|l| l.get("rev"))
        .and_then(|r| r.as_str());

    match (rev, expected_rev) {
        (Some(actual), Some(expected)) => {
            if actual != expected {
                return Err(format!(
                    "flake.lock input '{}' has rev '{}', expected '{}'",
                    input_name, actual, expected
                ));
            }
        }
        (Some(_), None) => {}
        (None, Some(expected)) => {
            return Err(format!(
                "flake.lock input '{}' has no 'rev' field, expected '{}'",
                input_name, expected
            ));
        }
        (None, None) => {}
    }

    Ok(())
}

fn git_push(repo: &Path, branch: &str) -> Result<(), String> {
    crate::git::git_push(repo, branch)
}

pub fn resume_local_commit(tx_id: &str, cfg: &WorkspaceConfig) -> Result<ActionResult, String> {
    resume_sync(tx_id, cfg, true)
}

pub fn resume_sync(
    transaction_id: &str,
    cfg: &WorkspaceConfig,
    no_push: bool,
) -> Result<ActionResult, String> {
    let journal = load_journal(transaction_id, cfg)?
        .ok_or_else(|| format!("Transaction '{}' not found", transaction_id))?;

    let _graph = crate::graph::discover_graph(cfg)?;

    if journal.actions.is_empty() {
        return Err(format!(
            "Transaction '{}' has no per-action journal (pre-refactor format). Cannot resume.",
            transaction_id
        ));
    }

    let mut push_results: BTreeMap<String, Result<(), String>> = BTreeMap::new();
    let mut commit_shas: BTreeMap<NodeId, String> = journal
        .nodes
        .iter()
        .filter_map(|(id, e)| e.commit_sha.clone().map(|sha| (id.clone(), sha)))
        .collect();
    let mut any_failed = false;
    let mut resume_journal = journal.clone();
    resume_journal.phase = JournalPhase::Planned;

    // Refuse resume if the worktree has files not in the expected set for pending Commit actions
    for entry in &resume_journal.actions {
        if entry.state == ActionState::Done {
            continue;
        }
        if matches!(entry.action, Action::Commit { .. }) {
            let current_files = collect_all_changed_files(
                resume_journal
                    .nodes
                    .get(&entry.node)
                    .map(|n| Path::new(&n.path))
                    .unwrap_or(Path::new(".")),
            )?;
            let unexpected: Vec<&String> = current_files
                .iter()
                .filter(|f| !entry.expected_files.contains(f))
                .collect();
            if !unexpected.is_empty() {
                return Err(format!(
                    "Resume refused: worktree for '{}' has {} unexpected file(s) not in the original commit set: {}. Expected {} files.",
                    entry.node,
                    unexpected.len(),
                    unexpected.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                    entry.expected_files.len()
                ));
            }
        }
    }

    // Resume from journal.actions, not a newly computed plan
    for entry in &resume_journal.actions.clone() {
        if entry.state == ActionState::Done {
            if matches!(entry.action, Action::Push { .. }) && entry.pushed {
                push_results.insert(entry.node.clone(), Ok(()));
            }
            continue;
        }

        match &entry.action {
            Action::Commit {
                node: node_id,
                message: _,
            } => {
                let current_files = collect_all_changed_files(
                    resume_journal
                        .nodes
                        .get(node_id)
                        .map(|n| Path::new(&n.path))
                        .unwrap_or(Path::new(".")),
                )?;
                if current_files.is_empty() {
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

                let node_path = resume_journal
                    .nodes
                    .get(node_id)
                    .map(|n| n.path.clone())
                    .unwrap_or_default();
                if let Some(e) = resume_journal
                    .actions
                    .iter_mut()
                    .find(|e| e.action_id == entry.action_id)
                {
                    e.state = ActionState::Pending;
                }
                write_journal(&resume_journal, cfg)?;

                let msg = if let Some(sha) = commit_shas.get(node_id) {
                    format!(
                        "chore(stitch): resume commit for {}\n\nPrevious: {}",
                        node_id, sha
                    )
                } else {
                    format!("chore(stitch): resume commit for {}", node_id)
                };

                git::git_add(Path::new(&node_path), &current_files)?;
                let trailed = crate::model::add_trailers(&msg, transaction_id, &cfg.workspace);
                git::git_commit(Path::new(&node_path), &trailed)?;
                let sha = git::git_head(Path::new(&node_path))?;
                commit_shas.insert(node_id.clone(), sha.clone());

                if let Some(e) = resume_journal
                    .actions
                    .iter_mut()
                    .find(|e| e.action_id == entry.action_id)
                {
                    e.state = ActionState::Done;
                    e.commit_sha = Some(sha.clone());
                }
                if let Some(n) = resume_journal.nodes.get_mut(node_id) {
                    n.commit_sha = Some(sha);
                }
                write_journal(&resume_journal, cfg)?;
            }
            Action::UpdateInputs {
                node: node_id,
                updates,
                ..
            } => {
                let node_path = resume_journal
                    .nodes
                    .get(node_id)
                    .map(|n| n.path.clone())
                    .unwrap_or_default();
                if let Some(e) = resume_journal
                    .actions
                    .iter_mut()
                    .find(|e| e.action_id == entry.action_id)
                {
                    e.state = ActionState::Running;
                }
                write_journal(&resume_journal, cfg)?;

                // Rerun update_flake_lock_input for each dependency, just like normal execution
                for update in updates {
                    let target_rev = commit_shas
                        .get(&update.dependency_node)
                        .or(update.target_rev.as_ref())
                        .ok_or_else(|| format!(
                            "Resume UpdateInputs for '{}': no commit SHA available for dependency '{}'",
                            node_id, update.dependency_node
                        ))?;
                    update_flake_lock_input(Path::new(&node_path), &update.input_name, target_rev)?;
                }

                let lock_path = Path::new(&node_path).join("flake.lock");
                if lock_path.exists() {
                    git::git_add(
                        Path::new(&node_path),
                        &[lock_path.to_string_lossy().to_string()],
                    )?;
                    let msg = format!("chore(inputs): resume sync for {}", node_id);
                    let trailed = crate::model::add_trailers(&msg, transaction_id, &cfg.workspace);
                    git::git_commit(Path::new(&node_path), &trailed)?;
                    let sha = git::git_head(Path::new(&node_path))?;
                    commit_shas.insert(node_id.clone(), sha.clone());

                    if let Some(e) = resume_journal
                        .actions
                        .iter_mut()
                        .find(|e| e.action_id == entry.action_id)
                    {
                        e.state = ActionState::Done;
                        e.commit_sha = Some(sha.clone());
                    }
                    if let Some(n) = resume_journal.nodes.get_mut(node_id) {
                        n.commit_sha = Some(sha);
                    }
                    write_journal(&resume_journal, cfg)?;
                } else {
                    if let Some(e) = resume_journal
                        .actions
                        .iter_mut()
                        .find(|e| e.action_id == entry.action_id)
                    {
                        e.state = ActionState::Done;
                    }
                    write_journal(&resume_journal, cfg)?;
                }
            }
            Action::Validate { node: node_id } => {
                // Validation is delegated to `tend plan/run`.
                // See docs/workflows/agent-check-flow.md for the recommended workflow.
                let node_path = resume_journal
                    .nodes
                    .get(node_id)
                    .map(|n| n.path.clone())
                    .unwrap_or_default();
                let status = std::process::Command::new("tend")
                    .args(["run", "--mode", "changed", "--phase", "verify"])
                    .current_dir(&node_path)
                    .status()
                    .map_err(|e| format!("Failed to run tend: {}", e))?;
                if !status.success() {
                    return Err(format!("Validation failed in '{}' during resume", node_id,));
                }

                if let Some(e) = resume_journal
                    .actions
                    .iter_mut()
                    .find(|e| e.action_id == entry.action_id)
                {
                    e.state = ActionState::Done;
                }
                write_journal(&resume_journal, cfg)?;
            }
            Action::Push { node: node_id } => {
                if entry.pushed {
                    push_results.insert(entry.node.clone(), Ok(()));
                    continue;
                }

                if no_push {
                    push_results.insert(node_id.clone(), Err("Skipped (--no-push)".to_string()));
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

                let node_path = resume_journal
                    .nodes
                    .get(node_id)
                    .map(|n| n.path.clone())
                    .unwrap_or_default();
                let branch = resume_journal
                    .nodes
                    .get(node_id)
                    .and_then(|n| n.branch.as_deref())
                    .unwrap_or("main");
                let result = git_push(Path::new(&node_path), branch);
                if let Err(ref e) = result {
                    push_results.insert(node_id.clone(), Err(e.clone()));
                    any_failed = true;
                    if let Some(je) = resume_journal
                        .actions
                        .iter_mut()
                        .find(|je| je.action_id == entry.action_id)
                    {
                        je.state = ActionState::Failed;
                        je.error = Some(e.clone());
                    }
                    resume_journal.phase = JournalPhase::Failed;
                    write_journal(&resume_journal, cfg)?;
                    break;
                }

                push_results.insert(node_id.clone(), Ok(()));
                if let Some(je) = resume_journal
                    .actions
                    .iter_mut()
                    .find(|je| je.action_id == entry.action_id)
                {
                    je.state = ActionState::Done;
                    je.pushed = true;
                }
                if let Some(n) = resume_journal.nodes.get_mut(node_id) {
                    n.pushed = true;
                }
                write_journal(&resume_journal, cfg)?;
            }
        }
    }

    if any_failed {
        return Ok(ActionResult {
            transaction_id: transaction_id.to_string(),
            created_commits: commit_shas,
            push_results,
            phase: JournalPhase::Failed,
        });
    }

    resume_journal.phase = JournalPhase::Completed;
    write_journal(&resume_journal, cfg)?;

    Ok(ActionResult {
        transaction_id: transaction_id.to_string(),
        created_commits: commit_shas,
        push_results,
        phase: JournalPhase::Completed,
    })
}

fn journal_dir(cfg: &WorkspaceConfig) -> Result<std::path::PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {}", e))?;
    let root_path = cfg
        .repos
        .iter()
        .find(|r| r.name == cfg.workspace || r.name == "phenix" || r.path == ".")
        .map(|r| r.resolved_path(cfg))
        .unwrap_or_else(|| cwd.clone());

    let dir = root_path.join(".stitch").join("transactions");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Create transaction dir: {}", e))?;
    Ok(dir)
}

fn write_journal(journal: &TransactionJournal, cfg: &WorkspaceConfig) -> Result<(), String> {
    let dir = journal_dir(cfg)?;
    let path = dir.join(format!("{}.json", journal.transaction_id));
    let content =
        serde_json::to_string_pretty(journal).map_err(|e| format!("Serialize journal: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("Write journal: {}", e))?;
    Ok(())
}

fn load_journal(
    transaction_id: &str,
    cfg: &WorkspaceConfig,
) -> Result<Option<TransactionJournal>, String> {
    let dir = journal_dir(cfg)?;
    let path = dir.join(format!("{}.json", transaction_id));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| format!("Read journal: {}", e))?;
    let journal: TransactionJournal =
        serde_json::from_str(&content).map_err(|e| format!("Parse journal: {}", e))?;
    Ok(Some(journal))
}

pub fn format_plan_output(plan: &ActionPlan, json_output: bool) -> String {
    if json_output {
        let action_list: Vec<serde_json::Value> = plan
            .actions
            .iter()
            .map(|a| match a {
                Action::Commit { node, .. } => {
                    serde_json::json!({ "type": "commit", "node": node })
                }
                Action::UpdateInputs { node, .. } => {
                    serde_json::json!({ "type": "update-inputs", "node": node })
                }
                Action::Validate { node } => {
                    serde_json::json!({ "type": "validate", "node": node })
                }
                Action::Push { node } => serde_json::json!({ "type": "push", "node": node }),
            })
            .collect();

        return serde_json::to_string_pretty(&serde_json::json!({
            "decision": if plan.blocked_reasons.is_empty() { "ready" } else { "blocked" },
            "root": plan.root,
            "transaction_id": plan.transaction_id,
            "nodes": plan.node_plans.iter().map(|(id, np)| {
                serde_json::json!({
                    "name": id,
                    "dirty": np.dirty,
                    "commit_required": np.needs_code_commit,
                    "sync_update_required": np.needs_input_sync,
                    "message": np.message,
                })
            }).collect::<Vec<_>>(),
            "actions": action_list,
            "blocked_reasons": plan.blocked_reasons,
        }))
        .unwrap_or_default();
    }

    let mut output = String::new();
    output.push_str(&format!("Sync Plan: {}\n", plan.transaction_id));
    output.push_str(&format!("Root: {}\n\n", plan.root));

    if !plan.blocked_reasons.is_empty() {
        output.push_str("BLOCKED:\n");
        for r in &plan.blocked_reasons {
            output.push_str(&format!("  - {}\n", r));
        }
        output.push('\n');
    }

    output.push_str("Action plan:\n");
    for (i, action) in plan.actions.iter().enumerate() {
        match action {
            Action::Commit { node, message } => {
                output.push_str(&format!("  {}. {}: commit\n", i + 1, node));
                output.push_str(&format!("       message: {}\n", message));
            }
            Action::UpdateInputs { node, updates, .. } => {
                output.push_str(&format!("  {}. {}: update-inputs\n", i + 1, node));
                for u in updates {
                    output.push_str(&format!(
                        "       {} -> {}\n",
                        u.input_name, u.dependency_node
                    ));
                }
            }
            Action::Validate { node } => {
                output.push_str(&format!("  {}. {}: validate\n", i + 1, node));
            }
            Action::Push { node } => {
                output.push_str(&format!("  {}. {}: push\n", i + 1, node));
            }
        }
    }

    output
}

pub fn format_result_output(result: &ActionResult, json_output: bool) -> String {
    if json_output {
        return serde_json::to_string_pretty(&serde_json::json!({
            "decision": match result.phase {
                JournalPhase::Completed => "completed",
                JournalPhase::Failed => "failed",
                JournalPhase::Committed => "committed",
                _ => "in_progress",
            },
            "transaction_id": result.transaction_id,
            "created_commits": result.created_commits,
            "push_results": result.push_results.iter().map(|(name, r)| {
                serde_json::json!({
                    "node": name,
                    "success": r.is_ok(),
                    "error": r.as_ref().err(),
                })
            }).collect::<Vec<_>>(),
        }))
        .unwrap_or_default();
    }

    let mut output = String::new();
    output.push_str(&format!("Transaction: {}\n", result.transaction_id));
    output.push_str(&format!("Phase: {}\n\n", result.phase));

    if !result.created_commits.is_empty() {
        output.push_str("Commits:\n");
        for (name, sha) in &result.created_commits {
            output.push_str(&format!("  {}: {}\n", name, sha));
        }
        output.push('\n');
    }

    if !result.push_results.is_empty() {
        output.push_str("Push results:\n");
        for (name, r) in &result.push_results {
            match r {
                Ok(()) => output.push_str(&format!("  {}: pushed\n", name)),
                Err(e) => output.push_str(&format!("  {}: FAILED - {}\n", name, e)),
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph;

    fn make_test_graph() -> WorkspaceGraph {
        let mut nodes = BTreeMap::new();
        for name in &["tools", "shell", "root"] {
            let dir = std::env::temp_dir().join(format!("__sync_test_{}", name));
            nodes.insert(
                name.to_string(),
                graph::FlakeNode {
                    id: name.to_string(),
                    name: name.to_string(),
                    path: dir,
                    remote: None,
                    branch: "main".to_string(),
                },
            );
        }
        let edges = vec![
            graph::DependencyEdge::new("shell", "tools", "tools"),
            graph::DependencyEdge::new("root", "shell", "shell"),
            graph::DependencyEdge::new("root", "tools", "tools"),
        ];
        WorkspaceGraph {
            root: "root".to_string(),
            nodes,
            edges,
        }
    }

    fn make_status(name: &str, dirty: bool) -> RepoStatus {
        RepoStatus {
            name: name.to_string(),
            path: format!("/tmp/{}", name),
            branch: "main".to_string(),
            is_dirty: dirty,
            status: RepoAvailability::Present,
            staged_count: if dirty { 1 } else { 0 },
            unstaged_count: 0,
            untracked_count: 0,
            ahead: None,
            behind: None,
        }
    }

    fn action_nodes(plan: &ActionPlan) -> Vec<String> {
        plan.actions.iter().map(|a| a.node().clone()).collect()
    }

    #[test]
    fn test_plan_sync_dirty_tools() {
        let graph = make_test_graph();
        let statuses = vec![
            make_status("tools", true),
            make_status("shell", false),
            make_status("root", false),
        ];
        let cfg = WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![],
            config_dir: None,
        };
        let plan = plan_sync(&graph, &statuses, &cfg).unwrap();
        let order = action_nodes(&plan);
        assert_eq!(
            order,
            vec!["tools", "shell", "root", "tools", "shell", "root", "tools", "shell", "root"]
        );
        let tools = plan.node_plans.get("tools").unwrap();
        assert!(tools.needs_code_commit);
        assert!(!tools.needs_input_sync);
        let shell = plan.node_plans.get("shell").unwrap();
        assert!(!shell.needs_code_commit);
        assert!(shell.needs_input_sync);
        let root = plan.node_plans.get("root").unwrap();
        assert!(!root.needs_code_commit);
        assert!(root.needs_input_sync);
        // Check action types
        assert!(matches!(plan.actions[0], Action::Commit { .. }));
        assert!(matches!(plan.actions[1], Action::UpdateInputs { .. }));
        assert!(matches!(plan.actions[2], Action::UpdateInputs { .. }));
        assert!(matches!(plan.actions[3], Action::Validate { .. }));
        assert!(matches!(plan.actions[4], Action::Validate { .. }));
        assert!(matches!(plan.actions[5], Action::Validate { .. }));
        assert!(matches!(plan.actions[6], Action::Push { .. }));
        assert!(matches!(plan.actions[7], Action::Push { .. }));
        assert!(matches!(plan.actions[8], Action::Push { .. }));
    }

    #[test]
    fn test_plan_sync_uses_exact_flake_input_name() {
        let mut graph = make_test_graph();
        for edge in &mut graph.edges {
            if edge.from == "shell" && edge.to == "tools" {
                edge.input_name = "tools-via-custom-input".to_string();
            }
        }
        let statuses = vec![
            make_status("tools", true),
            make_status("shell", false),
            make_status("root", false),
        ];
        let cfg = WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![],
            config_dir: None,
        };
        let plan = plan_sync(&graph, &statuses, &cfg).unwrap();
        let shell = plan.node_plans.get("shell").unwrap();
        assert_eq!(
            shell.dependencies_to_update[0].input_name,
            "tools-via-custom-input"
        );
    }

    #[test]
    fn test_plan_sync_dirty_shell() {
        let graph = make_test_graph();
        let statuses = vec![
            make_status("tools", false),
            make_status("shell", true),
            make_status("root", false),
        ];
        let cfg = WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![],
            config_dir: None,
        };
        let plan = plan_sync(&graph, &statuses, &cfg).unwrap();
        let order = action_nodes(&plan);
        assert_eq!(
            order,
            vec!["shell", "root", "shell", "root", "shell", "root"]
        );
        let shell = plan.node_plans.get("shell").unwrap();
        assert!(shell.needs_code_commit);
        assert!(!shell.needs_input_sync);
        let root = plan.node_plans.get("root").unwrap();
        assert!(!root.needs_code_commit);
        assert!(root.needs_input_sync);
    }

    #[test]
    fn test_plan_sync_dirty_root() {
        let graph = make_test_graph();
        let statuses = vec![
            make_status("tools", false),
            make_status("shell", false),
            make_status("root", true),
        ];
        let cfg = WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![],
            config_dir: None,
        };
        let plan = plan_sync(&graph, &statuses, &cfg).unwrap();
        let order = action_nodes(&plan);
        assert_eq!(order, vec!["root", "root", "root"]);
        let root = plan.node_plans.get("root").unwrap();
        assert!(root.needs_code_commit);
        assert!(!root.needs_input_sync);
    }

    #[test]
    fn test_plan_sync_dirty_tools_and_shell() {
        let graph = make_test_graph();
        let statuses = vec![
            make_status("tools", true),
            make_status("shell", true),
            make_status("root", false),
        ];
        let cfg = WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![],
            config_dir: None,
        };
        let plan = plan_sync(&graph, &statuses, &cfg).unwrap();
        let order = action_nodes(&plan);
        // tools: commit; shell: commit + update-inputs; root: update-inputs
        assert_eq!(
            order,
            vec![
                "tools", "shell", "shell", "root", "tools", "shell", "root", "tools", "shell",
                "root"
            ]
        );
        let tools = plan.node_plans.get("tools").unwrap();
        assert!(tools.needs_code_commit);
        assert!(!tools.needs_input_sync);
        let shell = plan.node_plans.get("shell").unwrap();
        assert!(shell.needs_code_commit);
        assert!(shell.needs_input_sync);
        let root = plan.node_plans.get("root").unwrap();
        assert!(!root.needs_code_commit);
        assert!(root.needs_input_sync);
    }

    #[test]
    fn test_plan_sync_no_dirty() {
        let graph = make_test_graph();
        let statuses = vec![
            make_status("tools", false),
            make_status("shell", false),
            make_status("root", false),
        ];
        let cfg = WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![],
            config_dir: None,
        };
        let plan = plan_sync(&graph, &statuses, &cfg).unwrap();
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn test_plan_push_order_equals_commit_order() {
        let graph = make_test_graph();
        let statuses = vec![
            make_status("tools", true),
            make_status("shell", true),
            make_status("root", true),
        ];
        let cfg = WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![],
            config_dir: None,
        };
        let plan = plan_sync(&graph, &statuses, &cfg).unwrap();
        // Push actions should be last, one per affected node in the same order
        let commit_nodes: Vec<&str> = plan
            .actions
            .iter()
            .filter_map(|a| {
                if matches!(a, Action::Commit { .. }) {
                    Some(a.node().as_str())
                } else {
                    None
                }
            })
            .collect();
        let push_nodes: Vec<&str> = plan
            .actions
            .iter()
            .filter_map(|a| {
                if matches!(a, Action::Push { .. }) {
                    Some(a.node().as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(commit_nodes, push_nodes);
    }

    #[test]
    fn test_update_flake_lock_input_fails_without_nix() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("__sync_test_flake_lock");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lock_content = r#"{
            "nodes": {
                "phenix-pins": {
                    "locked": {
                        "rev": "abc123",
                        "narHash": "sha256-xxx"
                    },
                    "original": {
                        "url": "github:user/pins"
                    }
                }
            }
        }"#;
        let mut f = std::fs::File::create(dir.join("flake.lock")).unwrap();
        f.write_all(lock_content.as_bytes()).unwrap();

        // Without nix available, this should fail (no direct fallback anymore)
        let result = update_flake_lock_input(
            &dir,
            "phenix-pins",
            "def4567890123456789012345678901234567890",
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("nix flake update") || err.contains("resumable"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_input_sync_message() {
        let updates = vec![InputUpdate {
            input_name: "tools".to_string(),
            dependency_node: "phenix-tools".to_string(),
            target_rev: Some("abc123".to_string()),
        }];
        let msg = input_sync_message("phenix-shell", &updates);
        assert!(msg.contains("phenix-tools"));
        assert!(msg.contains("abc123"));
    }

    #[test]
    fn test_generate_transaction_id() {
        let id = generate_transaction_id();
        assert!(id.starts_with("sync-"));
        assert!(id.len() > 10);
    }
}
