from pathlib import Path

for path in Path("crates").rglob("*.rs"):
    text = path.read_text()
    updated = text.replace("StepKind::Shell", "StepKind::Command")
    if updated != text:
        path.write_text(updated)

path = Path("crates/stitch/src/exec.rs")
text = path.read_text()

old_enum = '''pub enum StepKind {
    Shell {
        argv: Vec<String>,
    },
    Builtin {
'''
new_enum = '''pub enum StepKind {
    /// Execute an argv vector directly with `Command`; no shell is involved.
    Command {
        argv: Vec<String>,
    },
    Builtin {
'''
if text.count(old_enum) != 1:
    raise SystemExit(f"expected one StepKind enum, found {text.count(old_enum)}")
text = text.replace(old_enum, new_enum, 1)

condition_block = '''pub enum StepCondition {
    Always,
    Dirty,
    Staged,
    DirectlyChanged,
    DownstreamOnly,
    HasLockfile,
    HasChangedInputs,
}
'''
condition_replacement = condition_block + '''
#[derive(Debug, Clone, Copy)]
struct StepFacts {
    dirty: bool,
    staged: bool,
    has_lockfile: bool,
}

impl StepCondition {
    fn matches(self, node: &ExecutionNode, facts: StepFacts) -> bool {
        match self {
            Self::Always => true,
            Self::Dirty => facts.dirty,
            Self::Staged => facts.staged,
            Self::DirectlyChanged => node.directly_changed,
            Self::DownstreamOnly => node.downstream_only,
            Self::HasLockfile => facts.has_lockfile,
            Self::HasChangedInputs => node.downstream_only && facts.has_lockfile,
        }
    }
}
'''
if text.count(condition_block) != 1:
    raise SystemExit(f"expected one StepCondition block, found {text.count(condition_block)}")
text = text.replace(condition_block, condition_replacement, 1)

old_plan = '''    for node in &mut nodes {
        let is_dirty = is_node_dirty(&node.path);
        let is_staged = is_staged_check(&node.path);
        let has_lock = has_lockfile(&node.path);

        let applicable: Vec<ExecutionStep> = steps
            .iter()
            .filter(|step| {
                let cond = step.condition.unwrap_or(StepCondition::Always);
                match cond {
                    StepCondition::Always => true,
                    StepCondition::Dirty => is_dirty,
                    StepCondition::Staged => is_staged,
                    StepCondition::DirectlyChanged => node.directly_changed,
                    StepCondition::DownstreamOnly => node.downstream_only,
                    StepCondition::HasLockfile => has_lock,
                    StepCondition::HasChangedInputs => node.downstream_only && has_lock,
                }
            })
            .cloned()
            .collect();
'''
new_plan = '''    for node in &mut nodes {
        let facts = StepFacts {
            dirty: is_node_dirty(&node.path),
            staged: is_node_staged(&node.path),
            has_lockfile: has_lockfile(&node.path),
        };

        let applicable: Vec<ExecutionStep> = steps
            .iter()
            .filter(|step| {
                step.condition
                    .unwrap_or(StepCondition::Always)
                    .matches(node, facts)
            })
            .cloned()
            .collect();
'''
if text.count(old_plan) != 1:
    raise SystemExit(f"expected one inline condition block, found {text.count(old_plan)}")
text = text.replace(old_plan, new_plan, 1)

text = text.replace(
    '''fn is_staged_check(path: &Path) -> bool {
    is_node_staged(path)
}

''',
    "",
    1,
)
text = text.replace(
    'stderr: "Shell step has empty argv; provide a program or shell command"\n',
    'stderr: "Command step has empty argv; provide a program and arguments"\n',
    1,
)

text += '''

#[cfg(test)]
mod condition_tests {
    use super::*;

    fn node() -> ExecutionNode {
        ExecutionNode {
            name: "consumer".to_string(),
            path: PathBuf::from("."),
            role: None,
            layer: 3,
            directly_selected: false,
            directly_changed: true,
            downstream_only: true,
            steps: Vec::new(),
        }
    }

    #[test]
    fn named_step_predicates_express_domain_conditions() {
        let node = node();
        let facts = StepFacts {
            dirty: false,
            staged: true,
            has_lockfile: true,
        };

        assert!(StepCondition::Always.matches(&node, facts));
        assert!(!StepCondition::Dirty.matches(&node, facts));
        assert!(StepCondition::Staged.matches(&node, facts));
        assert!(StepCondition::DirectlyChanged.matches(&node, facts));
        assert!(StepCondition::DownstreamOnly.matches(&node, facts));
        assert!(StepCondition::HasLockfile.matches(&node, facts));
        assert!(StepCondition::HasChangedInputs.matches(&node, facts));
    }
}
'''

if "StepKind::Shell" in text or "    Shell {" in text:
    raise SystemExit("legacy shell variant remains in exec.rs")

path.write_text(text)
