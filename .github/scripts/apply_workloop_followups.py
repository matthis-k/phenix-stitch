from __future__ import annotations

import runpy
from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise RuntimeError(f"{label}: source pattern not found")
    return text.replace(old, new, 1)


primary_path = Path(".github/scripts/apply_workloop_lifecycle_invariants.py")
primary = primary_path.read_text()
primary = replace_once(
    primary,
    '    if count != 1:\n        raise RuntimeError(f"{label}: expected one match, found {count}")\n    return text.replace(old, new, 1)',
    '    if count < 1:\n        raise RuntimeError(f"{label}: expected a match")\n    return text.replace(old, new, 1)',
    "primary first-match helper",
)
primary_path.write_text(primary)
runpy.run_path(str(primary_path), run_name="__main__")

workloop_path = Path("crates/stitch/src/workloop.rs")
workloop = workloop_path.read_text()
workloop = replace_once(
    workloop,
    "        (LoopState::Open, LoopState::InSyncDev, _) => Ok(()),\n",
    "",
    "duplicate open transition",
)
workloop_path.write_text(workloop)

tools_path = Path("crates/stitch-mcp/src/tools.rs")
tools = tools_path.read_text()
tools = replace_once(
    tools,
    "        let checkpoint = match backend.checkpoint(&repo_path, &repo_name, message) {",
    '''        let detection = backend.detect(&repo_path).map_err(|e| {
            mk_err(
                ErrorKind::Internal,
                &format!("Backend detection failed: {e}"),
                &audit_id,
            )
        })?;
        let vcs_backend = workloop::VcsBackend::from_backend_state(&detection.state)
            .map_err(|e| mk_err(ErrorKind::Internal, &e, &audit_id))?;

        let checkpoint = match backend.checkpoint(&repo_path, &repo_name, message) {''',
    "MCP backend detection",
)
tools = replace_once(tools, "schema_version: 1,", "schema_version: 2,", "MCP schema")
tools = replace_once(
    tools,
    "backend: workloop::VcsBackend::Jj,",
    "backend: vcs_backend.clone(),",
    "MCP backend identity",
)
tools = replace_once(
    tools,
    '''                    last_evidence_id: None,
                },
                decisions:''',
    '''                    last_evidence_id: None,
                    verified_change_id: None,
                    verified_commit_id: None,
                },
                candidate: None,
                decisions:''',
    "MCP verification identity",
)
tools = replace_once(
    tools,
    '''        // Add/update repo ref in wallet
        let repo_ref =''',
    '''        wallet.backend = vcs_backend;

        // Add/update repo ref in wallet
        let repo_ref =''',
    "MCP wallet backend",
)
tools = replace_once(
    tools,
    '''        wallet.updated_at = workloop::Timestamp::now();
        wallet.revision += 1;
''',
    '''        wallet.invalidate_release();
        if let Err(e) = wallet.transition(
            workloop::LoopAction::DevSync,
            workloop::LoopState::InSyncDev,
        ) {
            return Err(mk_err(ErrorKind::Conflict, &e, &audit_id));
        }
''',
    "MCP authoritative transition",
)
tools_path.write_text(tools)
