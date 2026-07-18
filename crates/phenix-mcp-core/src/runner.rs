use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;
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
            match run_with_timeout(&mut cmd, timeout_dur) {
                Ok(result) => result,
                Err(error) => {
                    return Err(format!("Command timed out after {timeout}s: {error}"));
                }
            }
        } else {
            cmd.output()
                .map_err(|error| format!("Failed to execute command: {error}"))?
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

fn run_with_timeout(cmd: &mut Command, timeout: std::time::Duration) -> Result<Output, String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let start = std::time::Instant::now();
    let mut child = cmd.spawn().map_err(|error| format!("Spawn: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout pipe was not available".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr pipe was not available".to_string())?;
    let stdout_reader = read_stream(stdout, "stdout");
    let stderr_reader = read_stream(stderr, "stderr");

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Output {
                    status,
                    stdout: join_stream(stdout_reader, "stdout")?,
                    stderr: join_stream(stderr_reader, "stderr")?,
                });
            }
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_stream(stdout_reader, "stdout");
                let _ = join_stream(stderr_reader, "stderr");
                return Err("timeout".to_string());
            }
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_stream(stdout_reader, "stdout");
                let _ = join_stream(stderr_reader, "stderr");
                return Err(format!("Wait error: {error}"));
            }
        }
    }
}

fn read_stream(
    mut stream: impl Read + Send + 'static,
    name: &'static str,
) -> JoinHandle<Result<Vec<u8>, String>> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        stream
            .read_to_end(&mut output)
            .map_err(|error| format!("Read {name}: {error}"))?;
        Ok(output)
    })
}

fn join_stream(reader: JoinHandle<Result<Vec<u8>, String>>, name: &str) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| format!("{name} reader thread panicked"))?
}

pub fn validate_argv(argv: &[String]) -> Result<(), String> {
    if argv.is_empty() {
        return Err("Command must have at least one argument".to_string());
    }

    if argv.iter().any(|argument| argument.is_empty()) {
        return Err("Command arguments must not be empty strings".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_path_captures_stdout_and_stderr() {
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf stdout; printf stderr >&2".to_string(),
        ];
        let result = CommandRunner::new().run(&argv, None, Some(1)).unwrap();

        assert_eq!(result.stdout, "stdout");
        assert_eq!(result.stderr, "stderr");
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn timeout_path_terminates_long_running_command() {
        let argv = vec!["sh".to_string(), "-c".to_string(), "sleep 1".to_string()];
        let error = CommandRunner::new().run(&argv, None, Some(0)).unwrap_err();

        assert!(error.contains("timed out"));
    }
}
