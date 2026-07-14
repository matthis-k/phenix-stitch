from __future__ import annotations

import re
from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def regex_once(text: str, pattern: str, replacement: str, label: str) -> str:
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}")
    return updated


workloop_path = Path("crates/stitch/src/workloop.rs")
workloop = workloop_path.read_text()

workloop = replace_once(
    workloop,
    "use std::path::{Path, PathBuf};",
    "use std::fs::{self, OpenOptions};\nuse std::io::Write;\nuse std::path::{Path, PathBuf};",
    "workloop imports",
)

workloop = replace_once(
    workloop,
    "    /// Verification profile pointers — not full results.\n    pub verification: VerificationPointer,\n",
    "    /// Verification profile pointers — not full results.\n    pub verification: VerificationPointer,\n\n    /// The exact release candidate created from the verified source identity.\n    #[serde(default)]\n    pub candidate: Option<CandidateRef>,\n",
    "wallet candidate field",
)

workloop = replace_once(
    workloop,
    "}\n\n#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\npub enum VcsBackend",
    '''}\n\nimpl LoopWallet {\n    /// Apply a validated lifecycle transition and update all derived wallet metadata.\n    pub fn transition(&mut self, action: LoopAction, to: LoopState) -> Result<(), String> {\n        validate_state_transition(&self.state, &to, &action)?;\n        self.state = to;\n        self.next_valid_actions = valid_actions_for_state(&self.state);\n        self.updated_at = Timestamp::now();\n        self.revision = self\n            .revision\n            .checked_add(1)\n            .ok_or_else(|| "wallet revision overflow".to_string())?;\n        Ok(())\n    }\n\n    /// Any new development work invalidates release verification and candidate identity.\n    pub fn invalidate_release(&mut self) {\n        self.verification.release_status = CheckStatus::NotRun;\n        self.verification.last_evidence_id = None;\n        self.verification.verified_change_id = None;\n        self.verification.verified_commit_id = None;\n        self.candidate = None;\n        for repo in &mut self.repos {\n            repo.release_candidate_change_id = None;\n            repo.release_git_commit = None;\n        }\n    }\n}\n\n#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\npub enum VcsBackend''',
    "wallet impl",
)

workloop = replace_once(
    workloop,
    "pub enum VcsBackend {\n    #[serde(rename = \"jj\")]\n    Jj,\n    #[serde(rename = \"git\")]\n    Git,\n}\n",
    '''pub enum VcsBackend {\n    #[serde(rename = "jj")]\n    Jj,\n    #[serde(rename = "git")]\n    Git,\n}\n\nimpl VcsBackend {\n    pub fn from_backend_state(state: &BackendState) -> Result<Self, String> {\n        match state {\n            BackendState::JjColocated | BackendState::JjNative => Ok(Self::Jj),\n            BackendState::GitOnly => Ok(Self::Git),\n            BackendState::None => Err("no VCS backend detected".to_string()),\n        }\n    }\n}\n''',
    "backend conversion",
)

workloop = workloop.replace(
    "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\npub enum LoopAction",
    "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\npub enum LoopAction",
    1,
)
workloop = replace_once(
    workloop,
    "    #[serde(rename = \"finalize-dry-run\")]\n    FinalizeDryRun,\n",
    "    #[serde(rename = \"finalize-dry-run\")]\n    FinalizeDryRun,\n    #[serde(rename = \"create-release-candidate\")]\n    CreateReleaseCandidate,\n",
    "create candidate action",
)
workloop = replace_once(
    workloop,
    "            LoopAction::FinalizeDryRun => write!(f, \"finalize-dry-run\"),\n",
    "            LoopAction::FinalizeDryRun => write!(f, \"finalize-dry-run\"),\n            LoopAction::CreateReleaseCandidate => write!(f, \"create-release-candidate\"),\n",
    "display candidate action",
)

workloop = replace_once(
    workloop,
    "    /// Evidence ID from the last verification run (stored externally).\n    pub last_evidence_id: Option<String>,\n}\n",
    '''    /// Evidence ID from the last verification run (stored externally).\n    pub last_evidence_id: Option<String>,\n    /// Exact source identity covered by the release verification.\n    #[serde(default)]\n    pub verified_change_id: Option<String>,\n    #[serde(default)]\n    pub verified_commit_id: Option<String>,\n}\n\n#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\npub struct CandidateRef {\n    pub repo_name: String,\n    pub source_change_id: String,\n    pub source_commit_id: String,\n    pub change_id: String,\n    pub commit_id: String,\n    pub target_bookmark: String,\n}\n''',
    "verification identities",
)

workloop = replace_once(
    workloop,
    "        // ReadyToFinalize → ReleaseCandidate (finalize dry-run ran)\n        (LoopState::ReadyToFinalize, LoopState::ReleaseCandidate, LoopAction::FinalizeApply) => {\n            Ok(())\n        }\n",
    "        // ReadyToFinalize → ReleaseCandidate (candidate created from verified source)\n        (\n            LoopState::ReadyToFinalize,\n            LoopState::ReleaseCandidate,\n            LoopAction::CreateReleaseCandidate,\n        ) => Ok(()),\n",
    "candidate transition",
)
workloop = replace_once(
    workloop,
    "        // DirtyDev ↔ InSyncDev via dev-sync / edits\n        (LoopState::DirtyDev, LoopState::InSyncDev, LoopAction::DevSync) => Ok(()),\n        (LoopState::InSyncDev, LoopState::DirtyDev, _) => Ok(()),\n",
    '''        // Dev-sync establishes a fresh development fixed point and invalidates release state.\n        (\n            LoopState::Open\n            | LoopState::DirtyDev\n            | LoopState::InSyncDev\n            | LoopState::Blocked\n            | LoopState::ReadyToFinalize\n            | LoopState::ReleaseCandidate\n            | LoopState::ReleaseFixedPoint,\n            LoopState::InSyncDev,\n            LoopAction::DevSync,\n        ) => Ok(()),\n        (LoopState::InSyncDev, LoopState::DirtyDev, _) => Ok(()),\n''',
    "dev sync transition",
)
workloop = replace_once(
    workloop,
    "            LoopAction::FinalizeApply,\n            LoopAction::FinalizeDryRun,\n",
    "            LoopAction::CreateReleaseCandidate,\n            LoopAction::FinalizeDryRun,\n",
    "ready actions",
)

workloop = replace_once(
    workloop,
    "    fn resolve_jj(&self) -> Result<&Path, String> {",
    "    fn resolve_jj(&self) -> Result<PathBuf, String> {",
    "resolve jj signature",
)
workloop = replace_once(
    workloop,
    "                return Ok(path);",
    "                return Ok(path.clone());",
    "configured jj path",
)
workloop = replace_once(
    workloop,
    "                // Cache it\n                return Ok(Box::leak(Box::new(candidate)).as_path());",
    "                return Ok(candidate);",
    "path jj leak",
)
workloop = replace_once(
    workloop,
    "                return Ok(Box::leak(Box::new(PathBuf::from(candidate))).as_path());",
    "                return Ok(PathBuf::from(candidate));",
    "nix jj leak",
)
workloop = replace_once(
    workloop,
    "        let jj = self.resolve_jj()?.to_path_buf();",
    "        let jj = self.resolve_jj()?;",
    "run jj path",
)
workloop = replace_once(
    workloop,
    "        let _ = self.run_jj(repo, &[\"new\", \"@\"]);\n        let _ = self.run_jj(repo, &[\"describe\", \"@\", \"-m\", message]);",
    "        self.run_jj(repo, &[\"new\", \"@\"])?;\n        self.run_jj(repo, &[\"describe\", \"@\", \"-m\", message])?;",
    "checkpoint errors",
)

workloop = regex_once(
    workloop,
    r'''const WALLET_FILENAME: &str = "loop-wallet\.json";.*?/// List all known wallets\.''',
    '''const WALLET_FILENAME: &str = "loop-wallet.json";\n\n/// Where wallet files are stored relative to the workspace root.\nfn wallet_dir(workspace_root: &Path) -> PathBuf {\n    workspace_root.join(".stitch").join("loops")\n}\n\nfn validate_feature_id(feature: &str) -> Result<(), String> {\n    if feature.is_empty()\n        || !feature\n            .chars()\n            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))\n    {\n        return Err(format!(\n            "invalid feature id '{feature}': use ASCII letters, digits, '.', '-' or '_'"\n        ));\n    }\n    Ok(())\n}\n\nfn wallet_path(workspace_root: &Path, feature: &str) -> Result<PathBuf, String> {\n    validate_feature_id(feature)?;\n    Ok(wallet_dir(workspace_root).join(format!("{feature}-{WALLET_FILENAME}")))\n}\n\n/// Save a wallet atomically so an interrupted write cannot truncate durable state.\npub fn save_wallet(workspace_root: &Path, wallet: &LoopWallet) -> Result<(), String> {\n    let dir = wallet_dir(workspace_root);\n    fs::create_dir_all(&dir).map_err(|e| format!("failed to create wallet dir: {e}"))?;\n    let path = wallet_path(workspace_root, &wallet.feature)?;\n    let json = serde_json::to_vec_pretty(wallet)\n        .map_err(|e| format!("failed to serialize wallet: {e}"))?;\n\n    let mut temporary = None;\n    for attempt in 0..32_u32 {\n        let candidate = dir.join(format!(\n            ".{}.{}.{}.tmp",\n            wallet.loop_id,\n            std::process::id(),\n            attempt\n        ));\n        match OpenOptions::new().write(true).create_new(true).open(&candidate) {\n            Ok(file) => {\n                temporary = Some((candidate, file));\n                break;\n            }\n            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,\n            Err(error) => return Err(format!("failed to create temporary wallet: {error}")),\n        }\n    }\n\n    let (temporary_path, mut file) =\n        temporary.ok_or_else(|| "failed to allocate temporary wallet path".to_string())?;\n    let write_result = (|| -> Result<(), String> {\n        file.write_all(&json)\n            .map_err(|e| format!("failed to write temporary wallet: {e}"))?;\n        file.write_all(b"\\n")\n            .map_err(|e| format!("failed to terminate temporary wallet: {e}"))?;\n        file.sync_all()\n            .map_err(|e| format!("failed to sync temporary wallet: {e}"))?;\n        fs::rename(&temporary_path, &path)\n            .map_err(|e| format!("failed to replace wallet atomically: {e}"))?;\n        FileSync::sync_dir(&dir)?;\n        Ok(())\n    })();\n\n    if write_result.is_err() {\n        let _ = fs::remove_file(&temporary_path);\n    }\n    write_result\n}\n\nstruct FileSync;\n\nimpl FileSync {\n    fn sync_dir(dir: &Path) -> Result<(), String> {\n        let directory = fs::File::open(dir)\n            .map_err(|e| format!("failed to open wallet directory for sync: {e}"))?;\n        directory\n            .sync_all()\n            .map_err(|e| format!("failed to sync wallet directory: {e}"))\n    }\n}\n\n/// Load a wallet from disk.\npub fn load_wallet(workspace_root: &Path, feature: &str) -> Result<LoopWallet, String> {\n    let path = wallet_path(workspace_root, feature)?;\n    let json = fs::read_to_string(&path)\n        .map_err(|e| format!("failed to read wallet '{feature}': {e}"))?;\n    serde_json::from_str(&json).map_err(|e| format!("failed to parse wallet '{feature}': {e}"))\n}\n\n/// List all known wallets.''',
    "atomic wallet persistence",
)

# Populate new fields in existing test wallets.
workloop = workloop.replace(
    "                last_evidence_id: None,\n            },\n            decisions:",
    "                last_evidence_id: None,\n                verified_change_id: None,\n                verified_commit_id: None,\n            },\n            candidate: None,\n            decisions:",
)

# Append focused invariant tests before the end of the test module.
insert = '''\n    #[test]\n    fn transition_updates_revision_and_actions() {\n        let now = Timestamp::now();\n        let mut wallet = LoopWallet {\n            schema_version: 2,\n            loop_id: "loop-test".to_string(),\n            feature: "safe-feature".to_string(),\n            backend: VcsBackend::Git,\n            state: LoopState::Open,\n            repos: vec![],\n            verification: VerificationPointer {\n                dev_profile: None,\n                dev_status: CheckStatus::NotRun,\n                release_profile: None,\n                release_status: CheckStatus::NotRun,\n                last_evidence_id: None,\n                verified_change_id: None,\n                verified_commit_id: None,\n            },\n            candidate: None,\n            decisions: vec![],\n            blockers: vec![],\n            handoff: None,\n            next_valid_actions: valid_actions_for_state(&LoopState::Open),\n            created_at: now.clone(),\n            updated_at: now,\n            revision: 1,\n        };\n\n        wallet\n            .transition(LoopAction::DevSync, LoopState::InSyncDev)\n            .unwrap();\n        assert_eq!(wallet.state, LoopState::InSyncDev);\n        assert_eq!(wallet.revision, 2);\n        assert_eq!(\n            wallet.next_valid_actions,\n            valid_actions_for_state(&LoopState::InSyncDev)\n        );\n    }\n\n    #[test]\n    fn wallet_feature_cannot_escape_wallet_directory() {\n        let dir = tempfile::tempdir().unwrap();\n        let error = load_wallet(dir.path(), "../outside").unwrap_err();\n        assert!(error.contains("invalid feature id"));\n    }\n'''
idx = workloop.rfind("\n}")
if idx == -1:
    raise RuntimeError("test module closing brace not found")
workloop = workloop[:idx] + insert + workloop[idx:]
workloop_path.write_text(workloop)

main_path = Path("crates/stitch-cli/src/main.rs")
main = main_path.read_text()

main = replace_once(
    main,
    "            wallet.state = workloop::LoopState::InSyncDev;\n            wallet.updated_at = workloop::Timestamp::now();",
    "            wallet.invalidate_release();\n            wallet.transition(\n                workloop::LoopAction::DevSync,\n                workloop::LoopState::InSyncDev,\n            )?;",
    "checkpoint transition",
)

main = regex_once(
    main,
    r'''        LoopCliCommand::DevSync \{ feature, message \} => \{.*?\n        \}\n        LoopCliCommand::CreateRc''',
    '''        LoopCliCommand::DevSync { feature, message } => {\n            let message = message\n                .clone()\n                .unwrap_or_else(|| format!("dev-sync: {}", feature));\n            let backend = detect_backend_current_dir()?;\n            let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;\n            let detection = backend.detect(&cwd)?;\n            let snapshot = backend.snapshot(&cwd, feature)?;\n            if snapshot.has_conflicts {\n                return Err("cannot dev-sync a conflicted working copy".to_string());\n            }\n            let cp = backend.checkpoint(&cwd, feature, &message)?;\n            let vcs_backend = workloop::VcsBackend::from_backend_state(&detection.state)?;\n            let now = workloop::Timestamp::now();\n            let mut wallet = match workloop::load_wallet(&workspace_root, feature) {\n                Ok(wallet) => wallet,\n                Err(_) => workloop::LoopWallet {\n                    schema_version: 2,\n                    loop_id: format!("loop-{}", feature),\n                    feature: feature.clone(),\n                    backend: vcs_backend.clone(),\n                    state: workloop::LoopState::Open,\n                    repos: vec![],\n                    verification: workloop::VerificationPointer {\n                        dev_profile: None,\n                        dev_status: workloop::CheckStatus::NotRun,\n                        release_profile: None,\n                        release_status: workloop::CheckStatus::NotRun,\n                        last_evidence_id: None,\n                        verified_change_id: None,\n                        verified_commit_id: None,\n                    },\n                    candidate: None,\n                    decisions: vec![],\n                    blockers: vec![],\n                    handoff: None,\n                    next_valid_actions: workloop::valid_actions_for_state(\n                        &workloop::LoopState::Open,\n                    ),\n                    created_at: now.clone(),\n                    updated_at: now,\n                    revision: 1,\n                },\n            };\n\n            wallet.backend = vcs_backend;\n            let repo_ref = workloop::RepoLoopRef {\n                name: feature.clone(),\n                path: cwd,\n                workspace: None,\n                base_operation_id: snapshot.operation_id.clone(),\n                current_operation_id: cp.operation_id.clone(),\n                working_copy_change_id: cp.change_id.clone(),\n                working_copy_commit_id: snapshot.commit_id.clone(),\n                main_bookmark: snapshot.main_bookmark.clone(),\n                feature_bookmark: None,\n                release_candidate_change_id: None,\n                exported_git_commit: None,\n                release_git_commit: None,\n            };\n            if let Some(existing) = wallet.repos.iter_mut().find(|repo| repo.name == *feature) {\n                *existing = repo_ref;\n            } else {\n                wallet.repos.push(repo_ref);\n            }\n            wallet.invalidate_release();\n            wallet.transition(\n                workloop::LoopAction::DevSync,\n                workloop::LoopState::InSyncDev,\n            )?;\n            wallet.decisions.push(workloop::Decision {\n                title: format!("dev-sync: {}", cp.message),\n                rationale: format!("Dev-sync at change {}", cp.change_id),\n                outcome: workloop::DecisionOutcome::Accepted,\n                agent_id: None,\n                created_at: workloop::Timestamp::now(),\n            });\n            workloop::save_wallet(&workspace_root, &wallet)?;\n            println!("Dev-sync for '{}': snapshot + checkpoint done", feature);\n            Ok(())\n        }\n        LoopCliCommand::CreateRc''',
    "dev sync arm",
)

main = regex_once(
    main,
    r'''        LoopCliCommand::CreateRc \{ feature, target \} => \{.*?\n        \}\n        LoopCliCommand::FinalizeDryRun''',
    '''        LoopCliCommand::CreateRc { feature, target } => {\n            let mut wallet = workloop::load_wallet(&workspace_root, feature)?;\n            if wallet.state != workloop::LoopState::ReadyToFinalize {\n                return Err(format!(\n                    "feature '{}' must pass finalize-dry-run before creating a release candidate",\n                    feature\n                ));\n            }\n            let backend = detect_backend_for_wallet(&wallet)?;\n            let repo = wallet\n                .repos\n                .first()\n                .cloned()\n                .ok_or_else(|| "No repos in wallet".to_string())?;\n            let change = backend.current_change(&repo.path, &repo.name)?;\n            if wallet.verification.verified_change_id.as_deref() != Some(change.change_id.as_str())\n                || wallet.verification.verified_commit_id.as_deref()\n                    != Some(change.commit_id.as_str())\n            {\n                return Err("working copy changed after release verification; run finalize-dry-run again"\n                    .to_string());\n            }\n            let target_bookmark = target.clone().unwrap_or_else(|| "main".to_string());\n            let source_change_id = change.change_id.clone();\n            let source_commit_id = change.commit_id.clone();\n            let input = workloop::ReleaseInput {\n                repo_name: repo.name.clone(),\n                source_change_id: source_change_id.clone(),\n                target_bookmark: target_bookmark.clone(),\n                squash_message: None,\n            };\n            let rc = backend.create_release_candidate(&repo.path, input)?;\n            wallet.candidate = Some(workloop::CandidateRef {\n                repo_name: repo.name.clone(),\n                source_change_id,\n                source_commit_id,\n                change_id: rc.change_id.clone(),\n                commit_id: rc.commit_id.clone(),\n                target_bookmark,\n            });\n            if let Some(repo) = wallet.repos.first_mut() {\n                repo.release_candidate_change_id = Some(rc.change_id.clone());\n            }\n            wallet.transition(\n                workloop::LoopAction::CreateReleaseCandidate,\n                workloop::LoopState::ReleaseCandidate,\n            )?;\n            wallet.decisions.push(workloop::Decision {\n                title: format!("create-rc: {}", rc.commit_id),\n                rationale: format!("Release candidate created for '{}'", feature),\n                outcome: workloop::DecisionOutcome::Accepted,\n                agent_id: None,\n                created_at: workloop::Timestamp::now(),\n            });\n            workloop::save_wallet(&workspace_root, &wallet)?;\n            println!("Release candidate created: {}", rc.commit_id);\n            Ok(())\n        }\n        LoopCliCommand::FinalizeDryRun''',
    "create rc arm",
)

main = regex_once(
    main,
    r'''        LoopCliCommand::FinalizeDryRun \{ feature \} => \{.*?\n        \}\n        LoopCliCommand::FinalizeApply''',
    '''        LoopCliCommand::FinalizeDryRun { feature } => {\n            let mut wallet = workloop::load_wallet(&workspace_root, feature)?;\n            let backend = detect_backend_for_wallet(&wallet)?;\n            let repo = wallet\n                .repos\n                .first()\n                .cloned()\n                .ok_or_else(|| "No repos in wallet".to_string())?;\n            let snapshot = backend.snapshot(&repo.path, &repo.name)?;\n            if snapshot.has_conflicts || snapshot.has_divergence || snapshot.working_copy_is_stale {\n                return Err("release preflight rejected conflicted, divergent, or stale state"\n                    .to_string());\n            }\n            run_release_verification(&repo.path)?;\n            wallet.verification.release_profile = Some("full".to_string());\n            wallet.verification.release_status = workloop::CheckStatus::Passed;\n            wallet.verification.last_evidence_id =\n                Some(format!("tend:full:{}", snapshot.commit_id));\n            wallet.verification.verified_change_id = Some(snapshot.change_id.clone());\n            wallet.verification.verified_commit_id = Some(snapshot.commit_id.clone());\n            wallet.candidate = None;\n            wallet.transition(\n                workloop::LoopAction::FinalizeDryRun,\n                workloop::LoopState::ReadyToFinalize,\n            )?;\n            workloop::save_wallet(&workspace_root, &wallet)?;\n            println!("Finalize dry-run for '{}': Tend full profile passed", feature);\n            println!("  Verified commit: {}", snapshot.commit_id);\n            Ok(())\n        }\n        LoopCliCommand::FinalizeApply''',
    "finalize dry run arm",
)

main = regex_once(
    main,
    r'''        LoopCliCommand::FinalizeApply \{ feature, apply \} => \{.*?\n        \}\n        LoopCliCommand::Publish''',
    '''        LoopCliCommand::FinalizeApply { feature, apply } => {\n            if !*apply {\n                return Err("Must use --apply to finalize".to_string());\n            }\n            let mut wallet = workloop::load_wallet(&workspace_root, feature)?;\n            let candidate_ref = wallet\n                .candidate\n                .clone()\n                .ok_or_else(|| "No release candidate recorded in wallet".to_string())?;\n            let repo = wallet\n                .repos\n                .iter()\n                .find(|repo| repo.name == candidate_ref.repo_name)\n                .cloned()\n                .ok_or_else(|| "candidate repository is not tracked by wallet".to_string())?;\n            let backend = workloop::detect_backend(&repo.path)?;\n            let candidate = workloop::ReleaseCandidate {\n                repo_name: candidate_ref.repo_name.clone(),\n                change_id: candidate_ref.change_id.clone(),\n                commit_id: candidate_ref.commit_id.clone(),\n                exportable_git_commit_id: Some(candidate_ref.commit_id.clone()),\n                checks_status: workloop::CheckStatus::Passed,\n            };\n            let commit = backend.finalize_candidate(&repo.path, candidate)?;\n            if let Some(repo) = wallet\n                .repos\n                .iter_mut()\n                .find(|repo| repo.name == candidate_ref.repo_name)\n            {\n                repo.release_git_commit = commit.git_commit_id.clone();\n            }\n            wallet.transition(\n                workloop::LoopAction::FinalizeApply,\n                workloop::LoopState::ReleaseFixedPoint,\n            )?;\n            wallet.decisions.push(workloop::Decision {\n                title: format!("finalize: {}", commit.commit_id),\n                rationale: "Verified release candidate finalized".to_string(),\n                outcome: workloop::DecisionOutcome::Accepted,\n                agent_id: None,\n                created_at: workloop::Timestamp::now(),\n            });\n            workloop::save_wallet(&workspace_root, &wallet)?;\n            println!("Finalized: {}", commit.commit_id);\n            Ok(())\n        }\n        LoopCliCommand::Publish''',
    "finalize apply arm",
)

helper = '''\nfn run_release_verification(repo: &Path) -> Result<(), String> {\n    let output = std::process::Command::new("tend")\n        .args(["check", "--profile", "full", "--context", "local"])\n        .current_dir(repo)\n        .output()\n        .map_err(|e| format!("failed to run Tend release verification: {e}"))?;\n    if output.status.success() {\n        return Ok(());\n    }\n    let stderr = String::from_utf8_lossy(&output.stderr);\n    let stdout = String::from_utf8_lossy(&output.stdout);\n    Err(format!(\n        "Tend full profile failed:\\n{}{}",\n        stdout.trim(),\n        if stderr.trim().is_empty() {\n            String::new()\n        } else {\n            format!("\\n{}", stderr.trim())\n        }\n    ))\n}\n\n'''
main = replace_once(
    main,
    "/// Helper: detect backend for the current directory (for dev-sync)\n",
    helper + "/// Helper: detect backend for the current directory (for dev-sync)\n",
    "release verification helper",
)

main_path.write_text(main)
