use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::types::CommandResult;

pub struct CommandRunner;

impl Default for CommandRunner {
    fn default() -> Self {
        Self
    }
}

impl CommandRunner {
    pub fn new() -> Self {
        Self
    }

    pub fn run(
        &self,
        argv: &[String],
        cwd: Option<&Path>,
        timeout_seconds: Option<u64>,
    ) -> Result<CommandResult, String> {
        if argv.is_empty() {
            return Err("Empty command".to_string());
        }

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let start = Instant::now();

        let output = if let Some(timeout) = timeout_seconds {
            let timeout_dur = std::time::Duration::from_secs(timeout);
            match crate::runner::run_with_timeout(&mut cmd, timeout_dur) {
                Ok(result) => result,
                Err(e) => {
                    return Err(format!("Command timed out after {}s: {}", timeout, e));
                }
            }
        } else {
            cmd.output()
                .map_err(|e| format!("Failed to execute command: {}", e))?
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(CommandResult {
            command: argv.to_vec(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration_ms,
        })
    }

    pub fn run_with_workdir(
        &self,
        argv: &[String],
        workdir: &Path,
        timeout_seconds: Option<u64>,
    ) -> Result<CommandResult, String> {
        self.run(argv, Some(workdir), timeout_seconds)
    }
}

fn run_with_timeout(
    cmd: &mut Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    let start = std::time::Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("Spawn: {}", e))?;

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("Wait: {}", e))?;
                return Ok(output);
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timeout".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("Wait error: {}", e));
            }
        }
    }
}

pub fn validate_argv(argv: &[String]) -> Result<(), String> {
    if argv.is_empty() {
        return Err("Command must have at least one argument".to_string());
    }

    if argv.iter().any(|a| a.is_empty()) {
        return Err("Command arguments must not be empty strings".to_string());
    }

    Ok(())
}
