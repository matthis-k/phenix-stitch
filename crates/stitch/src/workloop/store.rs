//! Durable work-loop wallet storage.
//!
//! Wallet persistence is deliberately isolated from lifecycle policy and VCS
//! execution. Writes use create-new temporary files, fsync, and atomic rename.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::LoopWallet;

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
