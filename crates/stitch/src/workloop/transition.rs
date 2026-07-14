//! Pure work-loop transition policy.
//!
//! This module owns the lifecycle graph. It has no filesystem or VCS side
//! effects, which keeps transition review and exhaustive testing independent
//! from backend orchestration.

use super::{LoopAction, LoopState};

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

        // Publication can be retried after a publication-specific blocker.
        (LoopState::ReleaseFixedPoint, LoopState::Published, LoopAction::Publish) => Ok(()),
        (LoopState::Blocked, LoopState::Published, LoopAction::Publish) => Ok(()),

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
