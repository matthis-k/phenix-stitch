use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::{RepoAvailability, RepoStatus};

/// Index (staging area) status for a file in `git status --porcelain=v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexStatus {
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Ignored,
}

/// Worktree status for a file in `git status --porcelain=v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeStatus {
    Unmodified,
    Modified,
    Deleted,
    TypeChanged,
    Untracked,
    Ignored,
}

/// A typed representation of a single changed file from `git status --porcelain=v1`.
/// The two-letter code XY means X=index, Y=worktree.
#[derive(Debug, Clone)]
pub struct GitChange {
    pub path: String,
    pub old_path: Option<String>,
    pub index_status: IndexStatus,
    pub worktree_status: WorktreeStatus,
}

impl GitChange {
    pub fn is_staged(&self) -> bool {
        !matches!(
            self.index_status,
            IndexStatus::Unmodified | IndexStatus::Untracked | IndexStatus::Ignored
        )
    }

    pub fn is_unstaged(&self) -> bool {
        !matches!(
            self.worktree_status,
            WorktreeStatus::Unmodified | WorktreeStatus::Untracked | WorktreeStatus::Ignored
        )
    }

    pub fn is_untracked(&self) -> bool {
        matches!(self.index_status, IndexStatus::Untracked)
    }
}

#[derive(Debug, Clone)]
pub struct GitStatus {
    pub branch: String,
    pub changes: Vec<GitChange>,
}

impl GitStatus {
    pub fn is_dirty(&self) -> bool {
        !self.changes.is_empty()
    }

    pub fn staged_count(&self) -> usize {
        self.changes.iter().filter(|c| c.is_staged()).count()
    }

    pub fn unstaged_count(&self) -> usize {
        self.changes.iter().filter(|c| c.is_unstaged()).count()
    }

    pub fn untracked_count(&self) -> usize {
        self.changes.iter().filter(|c| c.is_untracked()).count()
    }

    pub fn staged_files(&self) -> Vec<&str> {
        self.changes
            .iter()
            .filter_map(|c| {
                if c.is_staged() {
                    Some(c.path.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn unstaged_files(&self) -> Vec<&str> {
        self.changes
            .iter()
            .filter_map(|c| {
                if c.is_unstaged() {
                    Some(c.path.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn all_files(&self) -> Vec<&str> {
        self.changes.iter().map(|c| c.path.as_str()).collect()
    }
}

/// A typed handle to a git repository.
#[derive(Debug, Clone)]
pub struct GitRepo {
    path: PathBuf,
}

impl GitRepo {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(path)
            .output()
            .map_err(|e| format!("git rev-parse: {}", e))?;
        if !output.status.success() {
            return Err(format!("Not a git repository: {}", path.display()));
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn status(&self) -> Result<GitStatus, String> {
        let branch = self.branch()?;
        let porcelain = self.porcelain()?;
        let changes = parse_porcelain(&porcelain);
        Ok(GitStatus { branch, changes })
    }

    pub fn branch(&self) -> Result<String, String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git branch: {}", e))?;
        if !output.status.success() {
            return Err(format!(
                "Not a git repo or no HEAD: {}",
                self.path.display()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn porcelain(&self) -> Result<String, String> {
        let output = Command::new("git")
            .args(["status", "--porcelain=v1"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git status: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git status --porcelain: {}", stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub fn head(&self) -> Result<String, String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git rev-parse HEAD: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git rev-parse HEAD: {}", stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn short_head(&self) -> Result<String, String> {
        let output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git rev-parse --short HEAD: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git rev-parse --short HEAD: {}", stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn add(&self, files: &[String]) -> Result<(), String> {
        let mut args = vec!["add", "--"];
        for f in files {
            args.push(f.as_str());
        }
        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git add: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git add: {}", stderr.trim()));
        }
        Ok(())
    }

    pub fn commit(&self, message: &str) -> Result<(), String> {
        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git commit: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git commit: {}", stderr.trim()));
        }
        Ok(())
    }

    pub fn push(&self, branch: &str) -> Result<(), String> {
        let output = Command::new("git")
            .args(["push", "origin", branch])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git push: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git push failed: {}", stderr.trim()));
        }
        Ok(())
    }

    pub fn ahead_count(&self) -> Result<usize, String> {
        let output = Command::new("git")
            .args(["rev-list", "--count", "@{upstream}..HEAD"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git rev-list: {}", e))?;
        if !output.status.success() {
            return Ok(0);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim().parse::<usize>().or(Ok(0))
    }

    pub fn has_merge_conflicts(&self) -> Result<bool, String> {
        let output = Command::new("git")
            .args(["grep", "-l", "^<<<<<<< \\|^=======\\|^>>>>>>> "])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git grep conflict: {}", e))?;
        Ok(output.status.success())
    }

    pub fn is_mid_merge(&self) -> Result<bool, String> {
        let output = Command::new("git")
            .args(["rev-parse", "--git-path", "MERGE_MSG"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git rev-parse --git-path MERGE_MSG: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "git rev-parse --git-path MERGE_MSG: {}",
                stderr.trim()
            ));
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let merge_msg_path = if Path::new(&raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            self.path.join(raw)
        };
        Ok(merge_msg_path.exists())
    }

    pub fn remote_url(&self, name: &str) -> Result<String, String> {
        let output = Command::new("git")
            .args(["remote", "get-url", name])
            .current_dir(&self.path)
            .output()
            .map_err(|e| format!("git remote get-url: {}", e))?;
        if !output.status.success() {
            return Err(format!("No remote '{}' in {}", name, self.path.display()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Parse `git status --porcelain=v1` output into a list of typed changes.
pub fn parse_porcelain(porcelain: &str) -> Vec<GitChange> {
    fn char_to_index(c: char) -> IndexStatus {
        match c {
            ' ' => IndexStatus::Unmodified,
            'A' => IndexStatus::Added,
            'M' => IndexStatus::Modified,
            'D' => IndexStatus::Deleted,
            'R' => IndexStatus::Renamed,
            'C' => IndexStatus::Copied,
            'T' => IndexStatus::TypeChanged,
            '?' => IndexStatus::Untracked,
            '!' => IndexStatus::Ignored,
            _ => IndexStatus::Unmodified,
        }
    }

    fn char_to_worktree(c: char) -> WorktreeStatus {
        match c {
            ' ' => WorktreeStatus::Unmodified,
            'M' => WorktreeStatus::Modified,
            'D' => WorktreeStatus::Deleted,
            'T' => WorktreeStatus::TypeChanged,
            '?' => WorktreeStatus::Untracked,
            '!' => WorktreeStatus::Ignored,
            _ => WorktreeStatus::Unmodified,
        }
    }

    let mut changes = Vec::new();
    for line in porcelain.lines() {
        let line = line.trim_end();
        if line.len() < 2 {
            continue;
        }

        let flags = &line[..2];
        let path_section = if line.len() > 3 { &line[3..] } else { "" };

        if flags == "!!" {
            continue;
        }

        let chars: Vec<char> = flags.chars().collect();
        let (x, y) = (chars[0], chars[1]);

        let (path, old_path) = if x == 'R' || x == 'C' {
            // Support both NUL-separated (-z) format and arrow format
            if let Some(nul_idx) = path_section.find('\0') {
                let old = &path_section[..nul_idx];
                let new = &path_section[nul_idx + 1..];
                (new.to_string(), Some(old.to_string()))
            } else if let Some(arrow_idx) = path_section.find(" -> ") {
                let old = &path_section[..arrow_idx];
                let new = &path_section[arrow_idx + 4..];
                (new.to_string(), Some(old.to_string()))
            } else {
                (path_section.to_string(), None)
            }
        } else {
            (path_section.to_string(), None)
        };

        changes.push(GitChange {
            path,
            old_path,
            index_status: char_to_index(x),
            worktree_status: char_to_worktree(y),
        });
    }
    changes
}

pub fn get_status(name: &str, repo_path: &Path) -> Result<RepoStatus, String> {
    let repo = GitRepo::open(repo_path)?;
    let git_status = repo.status()?;

    Ok(RepoStatus {
        name: name.to_string(),
        path: repo_path.to_string_lossy().to_string(),
        branch: git_status.branch.clone(),
        is_dirty: git_status.is_dirty(),
        status: RepoAvailability::Present,
        staged_count: git_status.staged_count(),
        unstaged_count: git_status.unstaged_count(),
        untracked_count: git_status.untracked_count(),
        ahead: None,
        behind: None,
    })
}

// Legacy free-function wrappers that delegate to GitRepo
pub fn git_branch(repo: &Path) -> Result<String, String> {
    GitRepo::open(repo)?.branch()
}

pub fn git_porcelain(repo: &Path) -> Result<String, String> {
    GitRepo::open(repo)?.porcelain()
}

pub fn git_head(repo: &Path) -> Result<String, String> {
    GitRepo::open(repo)?.head()
}

pub fn git_short_head(repo: &Path) -> Result<String, String> {
    GitRepo::open(repo)?.short_head()
}

pub fn git_add(repo: &Path, files: &[String]) -> Result<(), String> {
    GitRepo::open(repo)?.add(files)
}

pub fn git_commit(repo: &Path, message: &str) -> Result<(), String> {
    GitRepo::open(repo)?.commit(message)
}

pub fn git_push(repo: &Path, branch: &str) -> Result<(), String> {
    GitRepo::open(repo)?.push(branch)
}

pub fn git_diff_names(repo: &Path) -> Result<Vec<String>, String> {
    let git_status = GitRepo::open(repo)?.status()?;
    Ok(git_status
        .all_files()
        .into_iter()
        .map(|s| s.to_string())
        .collect())
}

pub fn git_diff_cached_names(repo: &Path) -> Result<Vec<String>, String> {
    let git_status = GitRepo::open(repo)?.status()?;
    Ok(git_status
        .staged_files()
        .into_iter()
        .map(|s| s.to_string())
        .collect())
}

pub fn has_merge_conflict_markers(repo: &Path) -> Result<bool, String> {
    GitRepo::open(repo)?.has_merge_conflicts()
}

pub fn git_ahead_count(repo: &Path, _branch: &str, _remote: &str) -> Result<usize, String> {
    GitRepo::open(repo)?.ahead_count()
}

pub fn is_mid_merge(repo: &Path) -> Result<bool, String> {
    GitRepo::open(repo)?.is_mid_merge()
}

pub fn git_remote(repo: &Path, name: &str) -> Result<String, String> {
    GitRepo::open(repo)?.remote_url(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_porcelain(input: &str) -> (usize, usize, usize) {
        let changes = parse_porcelain(input);
        let staged = changes.iter().filter(|c| c.is_staged()).count();
        let unstaged = changes.iter().filter(|c| c.is_unstaged()).count();
        let untracked = changes.iter().filter(|c| c.is_untracked()).count();
        (staged, unstaged, untracked)
    }

    #[test]
    fn test_parse_porcelain_clean() {
        let (staged, unstaged, untracked) = count_porcelain("");
        assert_eq!(staged, 0);
        assert_eq!(unstaged, 0);
        assert_eq!(untracked, 0);
    }

    #[test]
    fn test_parse_porcelain_dirty() {
        let input = " M modified.rs\nA  added.rs\n?? untracked.rs\n";
        let (staged, unstaged, untracked) = count_porcelain(input);
        assert_eq!(staged, 1);
        assert_eq!(unstaged, 1);
        assert_eq!(untracked, 1);
    }

    #[test]
    fn test_parse_porcelain_staged_unstaged_same_file() {
        let input = "MM modified.rs\n";
        let changes = parse_porcelain(input);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].is_staged());
        assert!(changes[0].is_unstaged());
        assert!(!changes[0].is_untracked());
    }

    #[test]
    fn test_parse_porcelain_all_staged() {
        let input = "M  staged_mod.rs\nA  added.rs\nD  deleted.rs\n";
        let changes = parse_porcelain(input);
        assert_eq!(changes.iter().filter(|c| c.is_staged()).count(), 3);
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn test_parse_porcelain_untracked_only() {
        let input = "?? new_file.rs\n?? another.txt\n";
        let changes = parse_porcelain(input);
        assert!(changes.iter().all(|c| c.is_untracked()));
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn parse_porcelain_worktree_modified_preserves_leading_space() {
        let input = " M modified.rs\n";
        let (staged, unstaged, untracked) = count_porcelain(input);
        assert_eq!(staged, 0, "expected 0 staged");
        assert_eq!(unstaged, 1, "expected 1 unstaged");
        assert_eq!(untracked, 0, "expected 0 untracked");
    }

    #[test]
    fn parse_porcelain_staged_modified() {
        let input = "M  modified.rs\n";
        let (staged, unstaged, untracked) = count_porcelain(input);
        assert_eq!(staged, 1, "expected 1 staged");
        assert_eq!(unstaged, 0, "expected 0 unstaged");
        assert_eq!(untracked, 0, "expected 0 untracked");
    }

    #[test]
    fn parse_porcelain_staged_and_unstaged_same_file() {
        let input = "MM modified.rs\n";
        let (staged, unstaged, untracked) = count_porcelain(input);
        assert_eq!(staged, 1, "expected 1 staged");
        assert_eq!(unstaged, 1, "expected 1 unstaged");
        assert_eq!(untracked, 0, "expected 0 untracked");
    }

    #[test]
    fn parse_porcelain_untracked() {
        let input = "?? new.rs\n";
        let (staged, unstaged, untracked) = count_porcelain(input);
        assert_eq!(staged, 0, "expected 0 staged");
        assert_eq!(unstaged, 0, "expected 0 unstaged");
        assert_eq!(untracked, 1, "expected 1 untracked");
    }

    #[test]
    fn parse_porcelain_ignored_skipped() {
        let input = "!! target/\n";
        let changes = parse_porcelain(input);
        assert_eq!(changes.len(), 0, "ignored entries should be skipped");
    }

    #[test]
    fn parse_porcelain_rename_arrow_format() {
        let input = "R  old.rs -> new.rs\n";
        let changes = parse_porcelain(input);
        assert_eq!(changes.len(), 1, "expected 1 change");
        assert_eq!(changes[0].path, "new.rs");
        assert_eq!(changes[0].old_path.as_deref(), Some("old.rs"));
        assert!(changes[0].is_staged(), "rename should be staged");
    }

    #[test]
    fn parse_porcelain_copy_arrow_format() {
        let input = "C  old.rs -> new.rs\n";
        let changes = parse_porcelain(input);
        assert_eq!(changes.len(), 1, "expected 1 change");
        assert_eq!(changes[0].path, "new.rs");
        assert_eq!(changes[0].old_path.as_deref(), Some("old.rs"));
        assert!(changes[0].is_staged(), "copy should be staged");
    }
}
