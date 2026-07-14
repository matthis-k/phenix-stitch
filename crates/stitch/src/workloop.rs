//! Work Loop — the JJ-backed feature development lifecycle.
//!
//! A "work loop" is one resumable feature/change-management session across
//! Phenix repos.  The wallet is the durable resume pointer; JJ owns the
//! per-repo mutable change graph; Stitch owns multi-repo orchestration.
//!
//! ## Architecture
//!
//! ```text
//! Wallet (lean resume pointer + decisions/blockers)
//!   ├── per-repo JJ change/op/bookmark IDs
//!   ├── verification pointers (dev profile + release profile)
//!   ├── decisions + blockers
//!   └── handoff packet (for agent handoff)
//!
//! LoopBackend trait
//!   ├── JjBackend  (primary)
//!   └── GitBackend (fallback)
//!
//! LoopStateMachine
//!   └── valid transitions between Open, DirtyDev, InSyncDev, ... Published
//! ```

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Wallet — the lean resume pointer
// ---------------------------------------------------------------------------

/// A work loop wallet.  Stores only pointers + decisions; recompute everything
/// else live from JJ / Stitch / Tend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopWallet {
    pub schema_version: u32,
    pub loop_id: String,
    pub feature: String,
    pub backend: VcsBackend,
    pub state: LoopState,

    /// Per-repo JJ identity snapshots.
    pub repos: Vec<RepoLoopRef>,

    /// Verification profile pointers — not full results.
    pub verification: VerificationPointer,

    /// The exact release candidate created from the verified source identity.
    #[serde(default)]
    pub candidate: Option<CandidateRef>,

    /// Decisions recorded during this loop (for handoff / audit).
    pub decisions: Vec<Decision>,
    /// Blockers that currently prevent forward progress.
    pub blockers: Vec<Blocker>,

    /// Optional handoff packet produced by `loop handoff`.
    pub handoff: Option<Handoff>,

    /// Actions that are valid to call right now, given current state.
    pub next_valid_actions: Vec<LoopAction>,

    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Monotonically increasing wallet revision (for optimistic locking).
    pub revision: u64,
}

impl LoopWallet {
    /// Apply a validated lifecycle transition and update all derived wallet metadata.
    pub fn transition(&mut self, action: LoopAction, to: LoopState) -> Result<(), String> {
        validate_state_transition(&self.state, &to, &action)?;
        self.state = to;
        self.next_valid_actions = valid_actions_for_state(&self.state);
        self.updated_at = Timestamp::now();
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| "wallet revision overflow".to_string())?;
        Ok(())
    }

    /// Any new development work invalidates release verification and candidate identity.
    pub fn invalidate_release(&mut self) {
        self.verification.release_status = CheckStatus::NotRun;
        self.verification.last_evidence_id = None;
        self.verification.verified_change_id = None;
        self.verification.verified_commit_id = None;
        self.candidate = None;
        for repo in &mut self.repos {
            repo.release_candidate_change_id = None;
            repo.release_git_commit = None;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VcsBackend {
    #[serde(rename = "jj")]
    Jj,
    #[serde(rename = "git")]
    Git,
}

impl VcsBackend {
    pub fn from_backend_state(state: &BackendState) -> Result<Self, String> {
        match state {
            BackendState::JjColocated | BackendState::JjNative => Ok(Self::Jj),
            BackendState::GitOnly => Ok(Self::Git),
            BackendState::None => Err("no VCS backend detected".to_string()),
        }
    }
}

/// Which VCS backend is detected for a repo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BackendState {
    /// `.jj/` and `.git/` both present (colocated)
    JjColocated,
    /// `.jj/` only
    JjNative,
    /// `.git/` only
    GitOnly,
    /// No VCS metadata found
    None,
}

/// A semantic action an agent or user can take.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LoopAction {
    #[serde(rename = "checkpoint")]
    Checkpoint,
    #[serde(rename = "dev-sync")]
    DevSync,
    #[serde(rename = "handoff")]
    Handoff,
    #[serde(rename = "finalize-dry-run")]
    FinalizeDryRun,
    #[serde(rename = "create-release-candidate")]
    CreateReleaseCandidate,
    #[serde(rename = "finalize-apply")]
    FinalizeApply,
    #[serde(rename = "publish")]
    Publish,
    #[serde(rename = "abandon")]
    Abandon,
}

impl std::fmt::Display for LoopAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopAction::Checkpoint => write!(f, "checkpoint"),
            LoopAction::DevSync => write!(f, "dev-sync"),
            LoopAction::Handoff => write!(f, "handoff"),
            LoopAction::FinalizeDryRun => write!(f, "finalize-dry-run"),
            LoopAction::CreateReleaseCandidate => write!(f, "create-release-candidate"),
            LoopAction::FinalizeApply => write!(f, "finalize-apply"),
            LoopAction::Publish => write!(f, "publish"),
            LoopAction::Abandon => write!(f, "abandon"),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-repo JJ identity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoLoopRef {
    pub name: String,
    pub path: PathBuf,

    /// JJ workspace identity (optional — single workspace by default).
    pub workspace: Option<String>,

    /// JJ operation ID at the start of the loop (baseline).
    pub base_operation_id: String,
    /// JJ operation ID at last snapshot.
    pub current_operation_id: String,

    /// JJ working-copy change ID (xxx format).
    pub working_copy_change_id: String,
    /// JJ working-copy commit ID (the materialized commit).
    pub working_copy_commit_id: String,

    // -- Publication pointers --
    /// The main bookmark name (usually "main").  Must not move until finalize.
    pub main_bookmark: String,
    /// Optional feature bookmark for stacked-work patterns.
    pub feature_bookmark: Option<String>,

    /// JJ change ID for the release candidate (created by finalize dry-run).
    pub release_candidate_change_id: Option<String>,

    // -- Git export identity (colocated mode) --
    /// The git commit ID that was last exported from JJ (in colocated mode).
    pub exported_git_commit: Option<String>,
    /// The git commit ID that is the release commit for this repo.
    pub release_git_commit: Option<String>,
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationPointer {
    /// Dev profile name (e.g. "dev-quick" or "ci-standard").
    pub dev_profile: Option<String>,
    pub dev_status: CheckStatus,

    /// Release profile name (e.g. "release-strict").
    pub release_profile: Option<String>,
    pub release_status: CheckStatus,

    /// Evidence ID from the last verification run (stored externally).
    pub last_evidence_id: Option<String>,
    /// Exact source identity covered by the release verification.
    #[serde(default)]
    pub verified_change_id: Option<String>,
    #[serde(default)]
    pub verified_commit_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateRef {
    pub repo_name: String,
    pub source_change_id: String,
    pub source_commit_id: String,
    pub change_id: String,
    pub commit_id: String,
    pub target_bookmark: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CheckStatus {
    #[serde(rename = "not-run")]
    NotRun,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "passed")]
    Passed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "skipped")]
    Skipped,
}

// ---------------------------------------------------------------------------
// Decisions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub title: String,
    pub rationale: String,
    pub outcome: DecisionOutcome,
    pub agent_id: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DecisionOutcome {
    #[serde(rename = "accepted")]
    Accepted,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "deferred")]
    Deferred,
}

// ---------------------------------------------------------------------------
// Blockers (structured, not free-text)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    pub kind: BlockerKind,
    pub repo: Option<String>,
    pub description: String,
    pub next_options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BlockerKind {
    #[serde(rename = "divergent-jj-change")]
    DivergentJjChange,
    #[serde(rename = "conflicted-change")]
    ConflictedChange,
    #[serde(rename = "stale-working-copy")]
    StaleWorkingCopy,
    #[serde(rename = "release-check-failed")]
    ReleaseCheckFailed,
    #[serde(rename = "lock-gitlink-mismatch")]
    LockGitlinkMismatch,
    #[serde(rename = "remote-missing-referenced-rev")]
    RemoteMissingReferencedRev,
    #[serde(rename = "secret-risk")]
    SecretRisk,
    #[serde(rename = "main-would-move-without-checks")]
    MainWouldMoveWithoutChecks,
    #[serde(rename = "publish-would-reference-unpushed-commit")]
    PublishWouldReferenceUnpushedCommit,
    #[serde(rename = "other")]
    Other,
}

// ---------------------------------------------------------------------------
// Loop State Machine
// ---------------------------------------------------------------------------

/// States a work loop can be in.  Transitions are enforced by the state
/// machine below.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LoopState {
    /// Initial state — loop created, no work done yet.
    #[serde(rename = "open")]
    Open,
    /// Working copy is dirty or changes haven't been checkpointed.
    #[serde(rename = "dirty-dev")]
    DirtyDev,
    /// All repos are internally coherent; dev-sync succeeded.
    #[serde(rename = "in-sync-dev")]
    InSyncDev,
    /// A blocker is preventing forward progress.
    #[serde(rename = "blocked")]
    Blocked,
    /// Release preflight checks passed; ready to create a candidate.
    #[serde(rename = "ready-to-finalize")]
    ReadyToFinalize,
    /// A release candidate has been created (by finalize dry-run).
    #[serde(rename = "release-candidate")]
    ReleaseCandidate,
    /// Strict checks passed; release is at fixed point.
    #[serde(rename = "release-fixed-point")]
    ReleaseFixedPoint,
    /// Published to remotes.
    #[serde(rename = "published")]
    Published,
    /// Abandoned — no longer active.
    #[serde(rename = "abandoned")]
    Abandoned,
}

impl std::fmt::Display for LoopState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopState::Open => write!(f, "open"),
            LoopState::DirtyDev => write!(f, "dirty-dev"),
            LoopState::InSyncDev => write!(f, "in-sync-dev"),
            LoopState::Blocked => write!(f, "blocked"),
            LoopState::ReadyToFinalize => write!(f, "ready-to-finalize"),
            LoopState::ReleaseCandidate => write!(f, "release-candidate"),
            LoopState::ReleaseFixedPoint => write!(f, "release-fixed-point"),
            LoopState::Published => write!(f, "published"),
            LoopState::Abandoned => write!(f, "abandoned"),
        }
    }
}

/// Validates a state transition.  Returns `Ok(())` if the transition
/// is allowed, or `Err` with a description of why not.
pub fn validate_state_transition(
    from: &LoopState,
    to: &LoopState,
    action: &LoopAction,
) -> Result<(), String> {
    match (from, to, action) {
        // Open → DirtyDev (edits started)
        (LoopState::Open, LoopState::DirtyDev, _) => Ok(()),

        // Dev-sync establishes a fresh development fixed point and invalidates release state.
        (
            LoopState::Open
            | LoopState::DirtyDev
            | LoopState::InSyncDev
            | LoopState::Blocked
            | LoopState::ReadyToFinalize
            | LoopState::ReleaseCandidate
            | LoopState::ReleaseFixedPoint,
            LoopState::InSyncDev,
            LoopAction::DevSync,
        ) => Ok(()),
        (LoopState::InSyncDev, LoopState::DirtyDev, _) => Ok(()),

        // InSyncDev → ReadyToFinalize (release preflight passed)
        (LoopState::InSyncDev, LoopState::ReadyToFinalize, LoopAction::FinalizeDryRun) => Ok(()),

        // ReadyToFinalize → ReleaseCandidate (candidate created from verified source)
        (
            LoopState::ReadyToFinalize,
            LoopState::ReleaseCandidate,
            LoopAction::CreateReleaseCandidate,
        ) => Ok(()),

        // ReleaseCandidate → ReleaseFixedPoint (strict checks converged)
        (LoopState::ReleaseCandidate, LoopState::ReleaseFixedPoint, LoopAction::FinalizeApply) => {
            Ok(())
        }

        // ReleaseFixedPoint → Published
        (LoopState::ReleaseFixedPoint, LoopState::Published, LoopAction::Publish) => Ok(()),

        // Any → Blocked
        (_, LoopState::Blocked, _) => Ok(()),

        // Blocked → DirtyDev (unblocked, back to work)
        (LoopState::Blocked, LoopState::DirtyDev, _) => Ok(()),

        // Any → Abandoned
        (_, LoopState::Abandoned, LoopAction::Abandon) => Ok(()),

        // Everything else is invalid
        _ => Err(format!(
            "Invalid transition: {} → {} via action {}",
            from, to, action
        )),
    }
}

/// Given a state, return the actions that are valid right now.
pub fn valid_actions_for_state(state: &LoopState) -> Vec<LoopAction> {
    match state {
        LoopState::Open => vec![LoopAction::DevSync, LoopAction::Abandon],
        LoopState::DirtyDev => vec![
            LoopAction::Checkpoint,
            LoopAction::DevSync,
            LoopAction::Handoff,
            LoopAction::Abandon,
        ],
        LoopState::InSyncDev => vec![
            LoopAction::Checkpoint,
            LoopAction::DevSync,
            LoopAction::Handoff,
            LoopAction::FinalizeDryRun,
            LoopAction::Abandon,
        ],
        LoopState::Blocked => vec![LoopAction::Handoff, LoopAction::Abandon],
        LoopState::ReadyToFinalize => vec![
            LoopAction::CreateReleaseCandidate,
            LoopAction::FinalizeDryRun,
            LoopAction::Handoff,
            LoopAction::Abandon,
        ],
        LoopState::ReleaseCandidate => vec![
            LoopAction::FinalizeApply,
            LoopAction::FinalizeDryRun,
            LoopAction::Handoff,
            LoopAction::Abandon,
        ],
        LoopState::ReleaseFixedPoint => vec![
            LoopAction::Publish,
            LoopAction::Handoff,
            LoopAction::Abandon,
        ],
        LoopState::Published | LoopState::Abandoned => vec![],
    }
}

// ---------------------------------------------------------------------------
// Handoff packet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub summary: String,
    pub changed_repos: Vec<String>,
    pub open_blockers: Vec<String>,
    pub recommended_next_action: Option<LoopAction>,
    pub created_by: String,
    pub created_at: Timestamp,
}

// ---------------------------------------------------------------------------
// Timestamp helper
// ---------------------------------------------------------------------------

/// RFC 3339 UTC timestamp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Timestamp(String);

impl Timestamp {
    pub fn now() -> Self {
        Self(crate::time::utc_timestamp())
    }
}

// ---------------------------------------------------------------------------
// LoopBackend trait — VCS abstraction
// ---------------------------------------------------------------------------

/// Result of detecting which backend a repo uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionResult {
    pub state: BackendState,
    pub jj_version: Option<String>,
}

/// A snapshot of a repo's current JJ/Git state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRef {
    pub repo_name: String,
    pub operation_id: String,
    pub change_id: String,
    pub commit_id: String,
    pub has_conflicts: bool,
    pub has_divergence: bool,
    pub working_copy_is_stale: bool,
    pub main_bookmark: String,
    pub main_commit_id: String,
}

/// A checkpoint reference (reproducible local state).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub repo_name: String,
    pub operation_id: String,
    pub change_id: String,
    pub message: String,
    pub created_at: Timestamp,
}

/// A change identity (for wallet/bookmarking).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRef {
    pub repo_name: String,
    pub change_id: String,
    pub commit_id: String,
    pub description: String,
    pub is_empty: bool,
}

/// Input for creating a release candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInput {
    pub repo_name: String,
    pub source_change_id: String,
    pub target_bookmark: String,
    pub squash_message: Option<String>,
}

/// A release candidate produced by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseCandidate {
    pub repo_name: String,
    pub change_id: String,
    pub commit_id: String,
    pub exportable_git_commit_id: Option<String>,
    pub checks_status: CheckStatus,
}

/// A finalized release commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseCommit {
    pub repo_name: String,
    pub change_id: String,
    pub commit_id: String,
    pub git_commit_id: Option<String>,
    pub bookmark_moved: bool,
}

/// A publish target with explicit path and bookmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishTarget {
    pub name: String,
    pub path: PathBuf,
    pub bookmark: String,
}

/// Refs to publish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRefs {
    #[serde(default)]
    #[deprecated(note = "use targets field instead")]
    pub repos: Vec<String>,
    #[serde(default)]
    #[deprecated(note = "use targets field instead")]
    pub main_bookmarks: Vec<String>,
    #[serde(default)]
    pub targets: Vec<PublishTarget>,
}

/// Result of a publish operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub pushed: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Abstract VCS backend.  Primary impl is `JjBackend`; `GitBackend`
/// is a fallback for repos that don't have JJ initialized.
pub trait LoopBackend: std::fmt::Debug {
    /// Detect which backend a repo is using (JJ colocated, JJ native, Git).
    fn detect(&self, repo: &Path) -> Result<DetectionResult, String>;

    /// Snapshot the current state of a repo.
    fn snapshot(&self, repo: &Path, name: &str) -> Result<SnapshotRef, String>;

    /// Create a checkpoint (resumable local development point).
    fn checkpoint(&self, repo: &Path, name: &str, message: &str) -> Result<CheckpointRef, String>;

    /// Get the current change identity for a repo.
    fn current_change(&self, repo: &Path, name: &str) -> Result<ChangeRef, String>;

    /// Create a release candidate from a source change, targeting a bookmark.
    fn create_release_candidate(
        &self,
        repo: &Path,
        input: ReleaseInput,
    ) -> Result<ReleaseCandidate, String>;

    /// Finalize a release candidate (squash, rebase, move bookmark).
    fn finalize_candidate(
        &self,
        repo: &Path,
        candidate: ReleaseCandidate,
    ) -> Result<ReleaseCommit, String>;

    /// Publish refs to remotes.
    fn publish(&self, refs: PublishRefs) -> Result<PublishResult, String>;
}

// ---------------------------------------------------------------------------
// JjBackend — primary implementation
// ---------------------------------------------------------------------------

/// JJ VCS backend.  Works in colocated mode by default.
#[derive(Debug, Default)]
pub struct JjBackend {
    /// Path to the `jj` binary.  Auto-detected if None.
    pub jj_bin: Option<PathBuf>,
}

impl JjBackend {
    pub fn new() -> Self {
        Self { jj_bin: None }
    }

    pub fn with_bin(path: PathBuf) -> Self {
        Self { jj_bin: Some(path) }
    }

    /// Resolve the `jj` binary path.
    fn resolve_jj(&self) -> Result<PathBuf, String> {
        if let Some(ref path) = self.jj_bin {
            if path.exists() {
                return Ok(path.clone());
            }
            return Err(format!("jj binary not found at: {}", path.display()));
        }
        // Auto-detect from PATH
        let paths = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("jj");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
        // Try common nix locations
        for candidate in [
            "/run/current-system/sw/bin/jj",
            "/nix/var/nix/profiles/default/bin/jj",
        ] {
            if Path::new(candidate).exists() {
                return Ok(PathBuf::from(candidate));
            }
        }
        Err("jj binary not found on PATH or common nix locations".to_string())
    }

    /// Run `jj` and return stdout as a trimmed string.
    fn run_jj(&self, repo: &Path, args: &[&str]) -> Result<String, String> {
        let jj = self.resolve_jj()?;
        let output = std::process::Command::new(&jj)
            .args(args)
            .env("JJ_EDITOR", "cat")
            .current_dir(repo)
            .output()
            .map_err(|e| format!("failed to run jj: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("jj {} failed: {}", args.join(" "), stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

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
}

impl LoopBackend for JjBackend {
    fn detect(&self, repo: &Path) -> Result<DetectionResult, String> {
        let has_jj = repo.join(".jj").is_dir();
        let has_git = repo.join(".git").exists(); // file or dir
        let version = if has_jj {
            self.run_jj(repo, &["--version"])
                .ok()
                .map(|v| v.lines().next().unwrap_or("unknown").to_string())
        } else {
            None
        };
        let state = match (has_jj, has_git) {
            (true, true) => BackendState::JjColocated,
            (true, false) => BackendState::JjNative,
            (false, true) => BackendState::GitOnly,
            (false, false) => BackendState::None,
        };
        Ok(DetectionResult {
            state,
            jj_version: version,
        })
    }

    fn snapshot(&self, repo: &Path, name: &str) -> Result<SnapshotRef, String> {
        let state = self.detect(repo)?;
        if state.state == BackendState::None || state.state == BackendState::GitOnly {
            return Err(format!(
                "repo '{}' at {} is not JJ-enabled (state: {:?})",
                name,
                repo.display(),
                state.state
            ));
        }

        let op_id = self.run_jj(
            repo,
            &["op", "log", "--color", "never", "-r", "@", "--no-graph"],
        )?;
        // Parse first non-empty line from op log output
        let op_id = op_id
            .lines()
            .find(|l| !l.is_empty())
            .unwrap_or("?")
            .to_string();

        let log = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                "@",
                "--no-graph",
                "--template",
                r#"change_id.shortest(12) ++ " " ++ commit_id.shortest(12)"#,
            ],
        )?;
        let parts: Vec<&str> = log.split_whitespace().collect();
        let change_id = parts.first().unwrap_or(&"?").to_string();
        let commit_id = parts.get(1).unwrap_or(&"?").to_string();

        // Check conflicts
        let has_conflicts = self
            .run_jj(
                repo,
                &[
                    "log",
                    "-r",
                    "@",
                    "--no-graph",
                    "--template",
                    r#"if(conflict, "yes", "no")"#,
                ],
            )
            .map(|s| s.trim() == "yes")
            .unwrap_or(false);

        // Check divergence
        let has_divergence = self
            .run_jj(repo, &["log", "-r", "divergent()", "--no-graph"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        // Main bookmark commit
        let main_commit = self
            .run_jj(
                repo,
                &[
                    "log",
                    "-r",
                    "main@origin",
                    "--no-graph",
                    "--template",
                    r#"commit_id.shortest(12)"#,
                ],
            )
            .unwrap_or_default();

        // Check stale working copy
        let wc_stale = self
            .run_jj(repo, &["workspace", "status"])
            .map(|s| s.contains("stale"))
            .unwrap_or(false);

        Ok(SnapshotRef {
            repo_name: name.to_string(),
            operation_id: op_id,
            change_id,
            commit_id,
            has_conflicts,
            has_divergence,
            working_copy_is_stale: wc_stale,
            main_bookmark: "main".to_string(),
            main_commit_id: main_commit.trim().to_string(),
        })
    }

    fn checkpoint(&self, repo: &Path, name: &str, message: &str) -> Result<CheckpointRef, String> {
        let _snap = self.snapshot(repo, name)?;

        // Create a new empty change on top of the current working copy,
        // so subsequent work starts fresh and the checkpoint is preserved.
        self.run_jj(repo, &["new", "@"])?;
        self.run_jj(repo, &["describe", "@", "-m", message])?;

        // Snapshot again to get the new change IDs
        let new_snap = self.snapshot(repo, name)?;

        Ok(CheckpointRef {
            repo_name: name.to_string(),
            operation_id: new_snap.operation_id,
            change_id: new_snap.change_id,
            message: message.to_string(),
            created_at: Timestamp::now(),
        })
    }

    fn current_change(&self, repo: &Path, name: &str) -> Result<ChangeRef, String> {
        let snap = self.snapshot(repo, name)?;
        let desc = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                "@",
                "--no-graph",
                "--template",
                r#"description.first_line()"#,
            ],
        )?;
        let is_empty = self
            .run_jj(
                repo,
                &[
                    "log",
                    "-r",
                    "@",
                    "--no-graph",
                    "--template",
                    r#"if(empty, "yes", "no")"#,
                ],
            )
            .map(|s| s.trim() == "yes")
            .unwrap_or(true);

        Ok(ChangeRef {
            repo_name: name.to_string(),
            change_id: snap.change_id,
            commit_id: snap.commit_id,
            description: desc,
            is_empty,
        })
    }

    fn create_release_candidate(
        &self,
        repo: &Path,
        input: ReleaseInput,
    ) -> Result<ReleaseCandidate, String> {
        let target_bookmark = &input.target_bookmark;
        let source_change_id = &input.source_change_id;

        // Step 1: Verify target bookmark exists
        self.run_jj(
            repo,
            &[
                "log",
                "-r",
                target_bookmark,
                "--no-graph",
                "-T",
                r#"commit_id.shortest(12)"#,
            ],
        )
        .map_err(|e| format!("target bookmark '{}' not found: {}", target_bookmark, e))?;

        // Step 2: Get source change description
        let description = self
            .run_jj(
                repo,
                &[
                    "log",
                    "-r",
                    source_change_id,
                    "--no-graph",
                    "-T",
                    r#"description.first_line()"#,
                ],
            )
            .map_err(|e| format!("source change '{}' not found: {}", source_change_id, e))?;

        // Step 3: Check source has no conflicts
        let has_conflicts = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                source_change_id,
                "--no-graph",
                "-T",
                r#"if(conflict, "yes", "no")"#,
            ],
        )?;
        if has_conflicts.trim() == "yes" {
            return Err(format!(
                "source change '{}' has conflicts; resolve before creating RC",
                source_change_id
            ));
        }

        // Step 4: Create RC change on target
        let rc_message = format!("[rc] {}", description.trim());
        self.run_jj(repo, &["new", target_bookmark, "-m", &rc_message])?;

        // Step 5: Get RC identity
        let rc_identity = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                "@",
                "--no-graph",
                "-T",
                r#"change_id.shortest(12) ++ " " ++ commit_id.shortest(12)"#,
            ],
        )?;
        let parts: Vec<&str> = rc_identity.split_whitespace().collect();
        let rc_change_id = parts
            .first()
            .ok_or("failed to parse RC change_id")?
            .to_string();
        let _rc_commit_id = parts
            .get(1)
            .ok_or("failed to parse RC commit_id")?
            .to_string();

        // Step 6: Rebase source onto target (OK if already on target)
        let rebase_result = self.run_jj(
            repo,
            &["rebase", "-r", source_change_id, "-d", target_bookmark],
        );
        if let Err(e) = &rebase_result {
            if !e.contains("onto itself") {
                return Err(format!("rebase failed (conflict?): {}", e));
            }
        }

        // Step 7: Squash source diff INTO the RC change
        self.run_jj(
            repo,
            &[
                "squash",
                "--from",
                source_change_id,
                "--into",
                &rc_change_id,
            ],
        )
        .map_err(|e| format!("squash into RC failed: {}", e))?;

        // Step 8: Update RC description if squash_message provided
        let _final_description = if let Some(ref msg) = input.squash_message {
            self.run_jj(repo, &["describe", "-r", &rc_change_id, "-m", msg])?;
            msg.clone()
        } else {
            rc_message
        };

        // Step 9: Final identity readback
        let final_identity = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                &rc_change_id,
                "--no-graph",
                "-T",
                r#"change_id.shortest(12) ++ " " ++ commit_id.shortest(12)"#,
            ],
        )?;
        let final_parts: Vec<&str> = final_identity.split_whitespace().collect();
        let final_change_id = final_parts
            .first()
            .ok_or("failed to parse final change_id")?
            .to_string();
        let final_commit_id = final_parts
            .get(1)
            .ok_or("failed to parse final commit_id")?
            .to_string();

        Ok(ReleaseCandidate {
            repo_name: input.repo_name,
            change_id: final_change_id,
            commit_id: final_commit_id.clone(),
            exportable_git_commit_id: Some(final_commit_id),
            checks_status: CheckStatus::NotRun,
        })
    }

    fn finalize_candidate(
        &self,
        repo: &Path,
        candidate: ReleaseCandidate,
    ) -> Result<ReleaseCommit, String> {
        let change_id = &candidate.change_id;

        // Step 1: Verify RC exists
        self.run_jj(
            repo,
            &[
                "log",
                "-r",
                change_id,
                "--no-graph",
                "-T",
                r#"commit_id.shortest(12)"#,
            ],
        )
        .map_err(|e| format!("release candidate '{}' not found: {}", change_id, e))?;

        // Step 2: Check conflicts
        let conflicts = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                change_id,
                "--no-graph",
                "-T",
                r#"if(conflict, "yes", "no")"#,
            ],
        )?;
        if conflicts.trim() == "yes" {
            return Err(format!("release candidate '{}' has conflicts", change_id));
        }

        // Step 3: Check empty
        let empty = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                change_id,
                "--no-graph",
                "-T",
                r#"if(empty, "yes", "no")"#,
            ],
        )?;
        if empty.trim() == "yes" {
            return Err(format!("release candidate '{}' is empty", change_id));
        }

        // Step 4: Move bookmark (target is always "main")
        self.run_jj(repo, &["bookmark", "set", "main", "-r", change_id])?;

        // Step 5: Verify
        let main_commit = self.run_jj(
            repo,
            &[
                "log",
                "-r",
                "main",
                "--no-graph",
                "-T",
                r#"commit_id.shortest(12)"#,
            ],
        )?;

        Ok(ReleaseCommit {
            repo_name: candidate.repo_name,
            change_id: change_id.clone(),
            commit_id: main_commit.trim().to_string(),
            git_commit_id: Some(main_commit.trim().to_string()),
            bookmark_moved: true,
        })
    }

    fn publish(&self, refs: PublishRefs) -> Result<PublishResult, String> {
        let mut pushed = Vec::new();
        let mut failed = Vec::new();

        for target in &refs.targets {
            let backend = JjBackend::new();
            let det = backend.detect(&target.path)?;
            match det.state {
                BackendState::JjColocated => {
                    // Belt-and-suspenders: export first, then git push
                    if let Err(e) = self.run_jj(&target.path, &["git", "export"]) {
                        failed.push((target.name.clone(), format!("jj git export failed: {}", e)));
                        continue;
                    }
                    match self.run_git(&target.path, &["push", "origin", &target.bookmark]) {
                        Ok(_) => pushed.push(target.name.clone()),
                        Err(e) => {
                            failed.push((target.name.clone(), format!("git push failed: {}", e)));
                        }
                    }
                }
                BackendState::JjNative => {
                    match self.run_jj(&target.path, &["git", "push", "-r", &target.bookmark]) {
                        Ok(_) => pushed.push(target.name.clone()),
                        Err(e) => {
                            failed
                                .push((target.name.clone(), format!("jj git push failed: {}", e)));
                        }
                    }
                }
                BackendState::GitOnly => {
                    match self.run_git(&target.path, &["push", "origin", &target.bookmark]) {
                        Ok(_) => pushed.push(target.name.clone()),
                        Err(e) => {
                            failed.push((target.name.clone(), format!("git push failed: {}", e)));
                        }
                    }
                }
                BackendState::None => {
                    failed.push((target.name.clone(), "no VCS backend detected".to_string()));
                }
            }
        }

        Ok(PublishResult { pushed, failed })
    }
}

// ---------------------------------------------------------------------------
// GitBackend — fallback for repos without JJ
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct GitBackend;

impl GitBackend {
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
}

impl LoopBackend for GitBackend {
    fn detect(&self, repo: &Path) -> Result<DetectionResult, String> {
        let has_git = repo.join(".git").exists();
        Ok(DetectionResult {
            state: if has_git {
                BackendState::GitOnly
            } else {
                BackendState::None
            },
            jj_version: None,
        })
    }

    fn snapshot(&self, repo: &Path, name: &str) -> Result<SnapshotRef, String> {
        let commit_id = self.run_git(repo, &["rev-parse", "HEAD"])?;
        let branch = self
            .run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap_or_default();
        let status = self.run_git(repo, &["status", "--porcelain=v1"])?;

        // Check for unmerged / conflicted paths
        let has_conflicts = status.lines().any(|line| {
            let s = line.trim();
            s.starts_with("U") || s == "AA" || s == "DD" || s == "UU"
        });

        Ok(SnapshotRef {
            repo_name: name.to_string(),
            operation_id: format!("git-snapshot-{}", &commit_id[..8]),
            change_id: commit_id.clone(),
            commit_id: commit_id.clone(),
            has_conflicts,
            has_divergence: false,
            working_copy_is_stale: false,
            main_bookmark: branch,
            main_commit_id: commit_id.clone(),
        })
    }

    fn checkpoint(&self, repo: &Path, name: &str, message: &str) -> Result<CheckpointRef, String> {
        self.run_git(repo, &["add", "-A"])?;
        self.run_git(repo, &["commit", "-m", &format!("checkpoint: {}", message)])?;
        let commit_id = self.run_git(repo, &["rev-parse", "HEAD"])?;

        Ok(CheckpointRef {
            repo_name: name.to_string(),
            operation_id: format!("checkpoint-{}", &commit_id[..8]),
            change_id: commit_id.clone(),
            message: format!("checkpoint: {}", message),
            created_at: Timestamp::now(),
        })
    }

    fn current_change(&self, repo: &Path, name: &str) -> Result<ChangeRef, String> {
        let commit_id = self.run_git(repo, &["rev-parse", "HEAD"])?;
        let description = self.run_git(repo, &["log", "-1", "--format=%s", "HEAD"])?;
        let status = self.run_git(repo, &["status", "--porcelain=v1"])?;
        let is_empty = status.trim().is_empty();

        Ok(ChangeRef {
            repo_name: name.to_string(),
            change_id: commit_id.clone(),
            commit_id,
            description,
            is_empty,
        })
    }

    fn create_release_candidate(
        &self,
        repo: &Path,
        input: ReleaseInput,
    ) -> Result<ReleaseCandidate, String> {
        let target = &input.target_bookmark;
        let source = &input.source_change_id;

        // Verify refs exist
        let target_commit = self
            .run_git(repo, &["rev-parse", "--verify", target])
            .map_err(|e| format!("target bookmark '{}' not found: {}", target, e))?;
        let source_commit = self
            .run_git(repo, &["rev-parse", "--verify", source])
            .map_err(|e| format!("source change '{}' not found: {}", source, e))?;

        // Get description
        let description = if let Some(ref msg) = input.squash_message {
            msg.clone()
        } else {
            self.run_git(repo, &["log", "-1", "--format=%s", source])?
        };

        // Create merge commit via merge-tree + commit-tree (no working tree mutation).
        // Uses --write-tree which is available in Git >= 2.34.
        let merge_tree_raw = self.run_git(
            repo,
            &[
                "merge-tree",
                "--write-tree",
                target_commit.trim(),
                source_commit.trim(),
            ],
        )?;
        let tree_oid = merge_tree_raw
            .lines()
            .next()
            .ok_or("merge-tree produced no output")?
            .trim()
            .to_string();

        // Validate that we got a plausible tree hash (40 hex chars)
        if tree_oid.len() != 40 || !tree_oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "merge-tree did not produce a valid tree OID, got: {}",
                tree_oid
            ));
        }

        let merge_commit = self.run_git(
            repo,
            &[
                "commit-tree",
                &tree_oid,
                "-p",
                target_commit.trim(),
                "-p",
                source_commit.trim(),
                "-m",
                &format!("release-candidate: {}", description),
            ],
        )?;
        let merge_commit = merge_commit.trim().to_string();

        // Tag with timestamp
        let tag_name = format!(
            "rc-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let _ = self.run_git(repo, &["tag", &tag_name, &merge_commit]);

        Ok(ReleaseCandidate {
            repo_name: input.repo_name,
            change_id: merge_commit.clone(),
            commit_id: merge_commit,
            exportable_git_commit_id: None,
            checks_status: CheckStatus::NotRun,
        })
    }

    fn finalize_candidate(
        &self,
        repo: &Path,
        candidate: ReleaseCandidate,
    ) -> Result<ReleaseCommit, String> {
        let commit_id = &candidate.commit_id;

        // Verify candidate exists and is a commit
        let obj_type = self.run_git(repo, &["cat-file", "-t", commit_id])?;
        if obj_type.trim() != "commit" {
            return Err(format!(
                "candidate '{}' is not a commit (type: {})",
                commit_id,
                obj_type.trim()
            ));
        }

        // Move main branch (use update-ref to avoid "checked out" restriction)
        self.run_git(repo, &["update-ref", "refs/heads/main", commit_id])?;

        // Verify
        let main_commit = self.run_git(repo, &["rev-parse", "main"])?;

        Ok(ReleaseCommit {
            repo_name: candidate.repo_name,
            change_id: commit_id.clone(),
            commit_id: main_commit.trim().to_string(),
            git_commit_id: Some(commit_id.clone()),
            bookmark_moved: true,
        })
    }

    fn publish(&self, refs: PublishRefs) -> Result<PublishResult, String> {
        let mut pushed = Vec::new();
        let mut failed = Vec::new();

        for target in &refs.targets {
            match self.run_git(&target.path, &["push", "origin", &target.bookmark]) {
                Ok(_) => pushed.push(target.name.clone()),
                Err(e) => failed.push((target.name.clone(), e)),
            }
        }

        Ok(PublishResult { pushed, failed })
    }
}

// ---------------------------------------------------------------------------
// Factory: pick the right backend for a repo
// ---------------------------------------------------------------------------

/// Detect and create the appropriate backend for a repo path.
pub fn detect_backend(repo: &Path) -> Result<Box<dyn LoopBackend>, String> {
    let jj = JjBackend::new();
    let det = jj.detect(repo)?;
    match det.state {
        BackendState::JjColocated | BackendState::JjNative => Ok(Box::new(jj)),
        BackendState::GitOnly => Ok(Box::new(GitBackend)),
        BackendState::None => Err(format!("no VCS backend detected at {}", repo.display())),
    }
}

// ---------------------------------------------------------------------------
// Wallet serialization / persistence
// ---------------------------------------------------------------------------

const WALLET_FILENAME: &str = "loop-wallet.json";

/// Where wallet files are stored relative to the workspace root.
fn wallet_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".stitch").join("loops")
}

fn validate_feature_id(feature: &str) -> Result<(), String> {
    if feature.is_empty()
        || !feature
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "invalid feature id '{feature}': use ASCII letters, digits, '.', '-' or '_'"
        ));
    }
    Ok(())
}

fn wallet_path(workspace_root: &Path, feature: &str) -> Result<PathBuf, String> {
    validate_feature_id(feature)?;
    Ok(wallet_dir(workspace_root).join(format!("{feature}-{WALLET_FILENAME}")))
}

/// Save a wallet atomically so an interrupted write cannot truncate durable state.
pub fn save_wallet(workspace_root: &Path, wallet: &LoopWallet) -> Result<(), String> {
    let dir = wallet_dir(workspace_root);
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create wallet dir: {e}"))?;
    let path = wallet_path(workspace_root, &wallet.feature)?;
    let json = serde_json::to_vec_pretty(wallet)
        .map_err(|e| format!("failed to serialize wallet: {e}"))?;

    let mut temporary = None;
    for attempt in 0..32_u32 {
        let candidate = dir.join(format!(
            ".{}.{}.{}.tmp",
            wallet.loop_id,
            std::process::id(),
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("failed to create temporary wallet: {error}")),
        }
    }

    let (temporary_path, mut file) =
        temporary.ok_or_else(|| "failed to allocate temporary wallet path".to_string())?;
    let write_result = (|| -> Result<(), String> {
        file.write_all(&json)
            .map_err(|e| format!("failed to write temporary wallet: {e}"))?;
        file.write_all(
            b"
",
        )
        .map_err(|e| format!("failed to terminate temporary wallet: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("failed to sync temporary wallet: {e}"))?;
        fs::rename(&temporary_path, &path)
            .map_err(|e| format!("failed to replace wallet atomically: {e}"))?;
        FileSync::sync_dir(&dir)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
}

struct FileSync;

impl FileSync {
    fn sync_dir(dir: &Path) -> Result<(), String> {
        let directory = fs::File::open(dir)
            .map_err(|e| format!("failed to open wallet directory for sync: {e}"))?;
        directory
            .sync_all()
            .map_err(|e| format!("failed to sync wallet directory: {e}"))
    }
}

/// Load a wallet from disk.
pub fn load_wallet(workspace_root: &Path, feature: &str) -> Result<LoopWallet, String> {
    let path = wallet_path(workspace_root, feature)?;
    let json =
        fs::read_to_string(&path).map_err(|e| format!("failed to read wallet '{feature}': {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("failed to parse wallet '{feature}': {e}"))
}

/// List all known wallets.
pub fn list_wallets(workspace_root: &Path) -> Result<Vec<String>, String> {
    let dir = wallet_dir(workspace_root);
    if !dir.is_dir() {
        return Ok(vec![]);
    }
    let mut wallets = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("failed to read wallet dir: {}", e))? {
        let entry = entry.map_err(|e| format!("bad entry: {}", e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(WALLET_FILENAME) {
            let feature = name.trim_end_matches(&format!("-{}", WALLET_FILENAME));
            wallets.push(feature.to_string());
        }
    }
    wallets.sort();
    Ok(wallets)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_machine_valid_transitions() {
        // Valid: Open → DirtyDev
        assert!(validate_state_transition(
            &LoopState::Open,
            &LoopState::DirtyDev,
            &LoopAction::Checkpoint
        )
        .is_ok());

        // Valid: DirtyDev → InSyncDev via DevSync
        assert!(validate_state_transition(
            &LoopState::DirtyDev,
            &LoopState::InSyncDev,
            &LoopAction::DevSync
        )
        .is_ok());

        // Invalid: Open → Published
        assert!(validate_state_transition(
            &LoopState::Open,
            &LoopState::Published,
            &LoopAction::Publish
        )
        .is_err());

        // Valid: ReleaseFixedPoint → Published
        assert!(validate_state_transition(
            &LoopState::ReleaseFixedPoint,
            &LoopState::Published,
            &LoopAction::Publish
        )
        .is_ok());

        // Valid: any → Blocked
        assert!(validate_state_transition(
            &LoopState::InSyncDev,
            &LoopState::Blocked,
            &LoopAction::DevSync
        )
        .is_ok());
    }

    #[test]
    fn test_valid_actions_for_states() {
        let actions = valid_actions_for_state(&LoopState::DirtyDev);
        assert!(actions.contains(&LoopAction::Checkpoint));
        assert!(actions.contains(&LoopAction::DevSync));
        assert!(actions.contains(&LoopAction::Handoff));

        let actions = valid_actions_for_state(&LoopState::InSyncDev);
        assert!(actions.contains(&LoopAction::FinalizeDryRun));
        assert!(!actions.contains(&LoopAction::Publish));

        let actions = valid_actions_for_state(&LoopState::ReleaseFixedPoint);
        assert!(actions.contains(&LoopAction::Publish));

        let actions = valid_actions_for_state(&LoopState::Published);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_timestamp_now() {
        let ts = Timestamp::now();
        let s = ts.0;
        // Basic format check: YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(s.len(), 20, "timestamp should be 20 chars: {}", s);
        assert!(s.ends_with('Z'), "timestamp should end with Z: {}", s);
        assert!(s.contains('T'), "timestamp should contain T: {}", s);
    }

    #[test]
    fn test_wallet_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let wallet = LoopWallet {
            schema_version: 1,
            loop_id: "test-001".to_string(),
            feature: "test-feature".to_string(),
            backend: VcsBackend::Jj,
            state: LoopState::Open,
            repos: vec![],
            verification: VerificationPointer {
                dev_profile: Some("dev-quick".to_string()),
                dev_status: CheckStatus::NotRun,
                release_profile: None,
                release_status: CheckStatus::NotRun,
                last_evidence_id: None,
                verified_change_id: None,
                verified_commit_id: None,
            },
            candidate: None,
            decisions: vec![],
            blockers: vec![],
            handoff: None,
            next_valid_actions: valid_actions_for_state(&LoopState::Open),
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            revision: 1,
        };

        save_wallet(dir.path(), &wallet).unwrap();
        let loaded = load_wallet(dir.path(), "test-feature").unwrap();
        assert_eq!(loaded.loop_id, "test-001");
        assert_eq!(loaded.feature, "test-feature");
        assert_eq!(loaded.state, LoopState::Open);
        assert_eq!(loaded.revision, 1);
    }

    #[test]
    fn test_detect_backend_on_nonexistent_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = detect_backend(dir.path());
        // No .git or .jj → should fail
        assert!(result.is_err());
    }

    #[test]
    fn test_jj_backend_auto_detect_path() {
        let jj = JjBackend::new();
        let result = jj.resolve_jj();
        // We can't guarantee jj is installed in test env, but the function
        // should either succeed or fail gracefully
        match result {
            Ok(path) => assert!(path.exists()),
            Err(msg) => assert!(msg.contains("not found")),
        }
    }

    #[test]
    fn test_jj_backend_detect_on_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let jj = JjBackend::new();
        let result = jj.detect(dir.path()).unwrap();
        // Temp dir has no .jj or .git
        assert_eq!(result.state, BackendState::None);
        assert!(result.jj_version.is_none());
    }

    #[test]
    fn test_blocker_kinds_serialize() {
        let blockers = vec![
            Blocker {
                kind: BlockerKind::DivergentJjChange,
                repo: Some("phenix-stitch".to_string()),
                description: "Divergent changes detected".to_string(),
                next_options: vec!["fix and dev-sync".to_string()],
            },
            Blocker {
                kind: BlockerKind::LockGitlinkMismatch,
                repo: Some("phenix".to_string()),
                description: "Lock/gitlink mismatch".to_string(),
                next_options: vec!["update submodules".to_string()],
            },
        ];
        let json = serde_json::to_string_pretty(&blockers).unwrap();
        assert!(json.contains("divergent-jj-change"));
        assert!(json.contains("lock-gitlink-mismatch"));
    }

    #[test]
    fn test_handoff_packet() {
        let handoff = Handoff {
            summary: "Updated pipeline semantics".to_string(),
            changed_repos: vec!["phenix-stitch".to_string()],
            open_blockers: vec![],
            recommended_next_action: Some(LoopAction::DevSync),
            created_by: "phenix-workflow".to_string(),
            created_at: Timestamp::now(),
        };
        let json = serde_json::to_string_pretty(&handoff).unwrap();
        assert!(json.contains("phenix-stitch"));
        assert!(json.contains("dev-sync"));
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn init_git_repo(path: &Path) {
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(path)
            .output()
            .unwrap();
        // First commit so HEAD exists
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap();
        // Rename default branch to main
        std::process::Command::new("git")
            .args(["branch", "-m", "main"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // GitBackend tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_git_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);

        let backend = GitBackend;
        let snap = backend.snapshot(&repo_path, "test-repo").unwrap();
        assert_eq!(snap.repo_name, "test-repo");
        // SHA is 40 hex chars
        assert_eq!(snap.commit_id.len(), 40);
        assert_eq!(snap.change_id.len(), 40);
        assert!(!snap.has_conflicts);
    }

    #[test]
    fn test_git_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);

        // Create a file to checkpoint
        std::fs::write(repo_path.join("test.txt"), "hello").unwrap();

        let backend = GitBackend;
        let cp = backend
            .checkpoint(&repo_path, "test-repo", "my checkpoint")
            .unwrap();
        assert_eq!(cp.repo_name, "test-repo");
        assert!(cp.message.contains("checkpoint: my checkpoint"));
        assert_eq!(cp.change_id.len(), 40);
    }

    #[test]
    fn test_git_current_change() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);

        let backend = GitBackend;
        let cc = backend.current_change(&repo_path, "test-repo").unwrap();
        assert_eq!(cc.repo_name, "test-repo");
        assert_eq!(cc.description, "initial");
        assert!(cc.is_empty); // initial commit has no uncommitted changes
    }

    #[test]
    fn test_git_create_rc() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);

        // Create a commit on main
        std::fs::write(repo_path.join("main.txt"), "main content").unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "main work"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Create feature branch with different content
        std::process::Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::fs::write(repo_path.join("feature.txt"), "feature content").unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "feature work"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Go back to main
        std::process::Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Get the source commit id
        let output = std::process::Command::new("git")
            .args(["rev-parse", "feature"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let source_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let backend = GitBackend;
        let input = ReleaseInput {
            repo_name: "test-repo".to_string(),
            source_change_id: source_id,
            target_bookmark: "main".to_string(),
            squash_message: None,
        };
        let rc = backend.create_release_candidate(&repo_path, input).unwrap();
        assert_eq!(rc.repo_name, "test-repo");
        assert!(!rc.commit_id.is_empty());
        assert_eq!(rc.commit_id.len(), 40);
    }

    #[test]
    fn test_git_finalize_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);

        // Create feature branch
        std::process::Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::fs::write(repo_path.join("feature.txt"), "feature").unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "feature work"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Create additional commit on feature (to have divergence from main)
        std::fs::write(repo_path.join("more.txt"), "more").unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "more work"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Go back to main
        std::process::Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Get source id
        let output = std::process::Command::new("git")
            .args(["rev-parse", "feature"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let source_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Create RC
        let backend = GitBackend;
        let input = ReleaseInput {
            repo_name: "test-repo".to_string(),
            source_change_id: source_id,
            target_bookmark: "main".to_string(),
            squash_message: Some("squashed release".to_string()),
        };
        let rc = backend.create_release_candidate(&repo_path, input).unwrap();

        // Finalize
        let finalized = backend.finalize_candidate(&repo_path, rc).unwrap();
        assert!(finalized.bookmark_moved);
        assert!(!finalized.commit_id.is_empty());

        // Verify main now points to the merge commit
        let main_commit = std::process::Command::new("git")
            .args(["rev-parse", "main"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let main_id = String::from_utf8_lossy(&main_commit.stdout)
            .trim()
            .to_string();
        assert_eq!(finalized.commit_id, main_id);
    }

    #[test]
    fn test_git_publish_no_remote() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);

        let refs: PublishRefs = serde_json::from_value(serde_json::json!({
            "targets": [{
                "name": "test-target",
                "path": repo_path,
                "bookmark": "main",
            }]
        }))
        .unwrap();
        let backend = GitBackend;
        let result = backend.publish(refs).unwrap();
        // No remote configured → should fail
        assert!(!result.failed.is_empty());
        assert!(result.pushed.is_empty());
    }

    // -----------------------------------------------------------------------
    // JjBackend tests (guarded — skip if `jj` not available)
    // -----------------------------------------------------------------------

    fn jj_available() -> bool {
        std::process::Command::new("which")
            .arg("jj")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn init_jj_repo(path: &Path) {
        std::process::Command::new("jj")
            .args(["git", "init", &path.to_string_lossy()])
            .output()
            .unwrap();
        // Set user config via git (jj uses git config in colocated mode)
        std::process::Command::new("git")
            .args([
                "-C",
                &path.to_string_lossy(),
                "config",
                "user.email",
                "test@test",
            ])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-C", &path.to_string_lossy(), "config", "user.name", "test"])
            .output()
            .unwrap();

        // Create initial commit and main bookmark (JJ 0.42+ doesn't auto-create main)
        std::process::Command::new("jj")
            .args(["new", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("jj")
            .args(["bookmark", "create", "main", "-r", "@-"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    #[test]
    fn test_jj_create_rc() {
        if !jj_available() {
            eprintln!("jj not available, skipping");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_jj_repo(&repo_path);
        // Write a file so the feature change has actual content
        std::fs::write(repo_path.join("test.txt"), "feature content").unwrap();

        // Create a feature change
        std::process::Command::new("jj")
            .args(["describe", "-m", "feature work"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Get the source change ID (the change we just created)
        let out = std::process::Command::new("jj")
            .args([
                "log",
                "-r",
                "@",
                "--no-graph",
                "-T",
                "change_id.shortest(12)",
            ])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let source_id = String::from_utf8_lossy(&out.stdout).trim().to_string();

        // Go back to main
        std::process::Command::new("jj")
            .args(["new", "main"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let backend = JjBackend::new();
        let input = ReleaseInput {
            repo_name: "test-repo".to_string(),
            source_change_id: source_id.clone(),
            target_bookmark: "main".to_string(),
            squash_message: None,
        };
        let rc = backend.create_release_candidate(&repo_path, input).unwrap();
        assert_eq!(rc.repo_name, "test-repo");
        assert!(!rc.commit_id.is_empty());
        // Should be at least 8 chars (shortest(12) prefix)
        assert!(rc.commit_id.len() >= 8);
    }

    #[test]
    fn test_jj_finalize_candidate() {
        if !jj_available() {
            eprintln!("jj not available, skipping");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_jj_repo(&repo_path);

        // Create a feature change
        // Write a file so the feature change has actual content
        std::fs::write(repo_path.join("test.txt"), "feature content").unwrap();

        std::process::Command::new("jj")
            .args(["describe", "-m", "feature work"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let out = std::process::Command::new("jj")
            .args([
                "log",
                "-r",
                "@",
                "--no-graph",
                "-T",
                "change_id.shortest(12)",
            ])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let source_id = String::from_utf8_lossy(&out.stdout).trim().to_string();

        // Go back to main
        std::process::Command::new("jj")
            .args(["new", "main"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let backend = JjBackend::new();
        let input = ReleaseInput {
            repo_name: "test-repo".to_string(),
            source_change_id: source_id,
            target_bookmark: "main".to_string(),
            squash_message: Some("squashed".to_string()),
        };
        let rc = backend.create_release_candidate(&repo_path, input).unwrap();

        let finalized = backend.finalize_candidate(&repo_path, rc).unwrap();
        assert!(finalized.bookmark_moved);
        assert!(!finalized.commit_id.is_empty());
    }

    #[test]
    fn test_jj_publish_no_remote() {
        if !jj_available() {
            eprintln!("jj not available, skipping");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_jj_repo(&repo_path);

        let refs: PublishRefs = serde_json::from_value(serde_json::json!({
            "targets": [{
                "name": "test-target",
                "path": repo_path,
                "bookmark": "main",
            }]
        }))
        .unwrap();
        let backend = JjBackend::new();
        let result = backend.publish(refs).unwrap();
        // JjNative repo without remote → should fail
        assert!(!result.failed.is_empty());
        assert!(result.pushed.is_empty());
    }
    #[test]
    fn transition_updates_revision_and_actions() {
        let now = Timestamp::now();
        let mut wallet = LoopWallet {
            schema_version: 2,
            loop_id: "loop-test".to_string(),
            feature: "safe-feature".to_string(),
            backend: VcsBackend::Git,
            state: LoopState::Open,
            repos: vec![],
            verification: VerificationPointer {
                dev_profile: None,
                dev_status: CheckStatus::NotRun,
                release_profile: None,
                release_status: CheckStatus::NotRun,
                last_evidence_id: None,
                verified_change_id: None,
                verified_commit_id: None,
            },
            candidate: None,
            decisions: vec![],
            blockers: vec![],
            handoff: None,
            next_valid_actions: valid_actions_for_state(&LoopState::Open),
            created_at: now.clone(),
            updated_at: now,
            revision: 1,
        };

        wallet
            .transition(LoopAction::DevSync, LoopState::InSyncDev)
            .unwrap();
        assert_eq!(wallet.state, LoopState::InSyncDev);
        assert_eq!(wallet.revision, 2);
        assert_eq!(
            wallet.next_valid_actions,
            valid_actions_for_state(&LoopState::InSyncDev)
        );
    }

    #[test]
    fn wallet_feature_cannot_escape_wallet_directory() {
        let dir = tempfile::tempdir().unwrap();
        let error = load_wallet(dir.path(), "../outside").unwrap_err();
        assert!(error.contains("invalid feature id"));
    }
}
