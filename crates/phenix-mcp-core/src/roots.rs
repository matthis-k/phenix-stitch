use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRoot {
    pub uri: String,
    pub path: PathBuf,
    pub writable: bool,
}

impl McpRoot {
    pub fn new(path: PathBuf, writable: bool) -> Self {
        let canonical = if path.is_absolute() {
            dunce::canonicalize(&path).unwrap_or(path.clone())
        } else {
            path.clone()
        };
        let uri = format!("file://{}", canonical.display());
        Self {
            uri,
            path: canonical,
            writable,
        }
    }

    pub fn contains(&self, target: &Path) -> bool {
        is_path_inside_root(target, self)
    }
}

pub struct RootValidator {
    roots: Vec<McpRoot>,
}

impl RootValidator {
    pub fn new(roots: Vec<McpRoot>) -> Self {
        Self { roots }
    }

    pub fn validate_path(&self, path: &Path) -> Result<(), String> {
        if path.is_absolute() {
            for root in &self.roots {
                if root.contains(path) {
                    return Ok(());
                }
            }
            return Err(format!(
                "Path '{}' is outside all declared roots",
                path.display()
            ));
        }
        Ok(())
    }

    pub fn resolve_in_root(&self, path: &Path, root: &McpRoot) -> Result<PathBuf, String> {
        let resolved = if path.is_relative() {
            root.path.join(path)
        } else {
            path.to_path_buf()
        };

        if !root.contains(&resolved) {
            return Err(format!(
                "Path '{}' resolves outside root '{}'",
                path.display(),
                root.path.display()
            ));
        }

        Ok(resolved)
    }

    pub fn roots(&self) -> &[McpRoot] {
        &self.roots
    }
}

mod dunce {
    use std::path::{Component, Path, PathBuf};

    pub fn canonicalize(path: &Path) -> Result<PathBuf, std::io::Error> {
        if path.exists() {
            path.canonicalize()
        } else {
            Ok(normalize(path))
        }
    }

    /// Lexically normalize a path, resolving `..` and `.` components.
    /// Does not touch the filesystem.
    pub fn normalize(path: &Path) -> PathBuf {
        let mut components: Vec<Component> = Vec::new();
        for component in path.components() {
            match component {
                Component::ParentDir => {
                    if components
                        .last()
                        .is_some_and(|c| matches!(c, Component::Normal(_)))
                    {
                        components.pop();
                    } else if components
                        .last()
                        .is_none_or(|c| !matches!(c, Component::RootDir))
                    {
                        components.push(component);
                    }
                }
                Component::CurDir => {}
                other => components.push(other),
            }
        }
        components.iter().collect()
    }
}

/// Lexically check if `path` starts with `base`, normalizing both.
fn path_starts_with(path: &Path, base: &Path) -> bool {
    let normal_path = dunce::normalize(path);
    let normal_base = dunce::normalize(base);
    normal_path.starts_with(&normal_base)
}

pub fn is_path_inside_root(path: &Path, root: &McpRoot) -> bool {
    if let Ok(canonical) = dunce::canonicalize(path) {
        canonical.starts_with(&root.path)
    } else {
        path_starts_with(path, &root.path)
    }
}
