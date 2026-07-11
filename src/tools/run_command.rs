use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::process::Command;

pub struct RunCommand;

impl super::Tool for RunCommand {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command on the local machine and return its combined stdout/stderr output \
         along with the exit code. Use with caution: commands run with the same privileges as the agent process."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute (passed to `sh -c`)"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory to run the command in (default: current working directory)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Maximum number of seconds to allow the command to run before it is killed (default: 30)"
                }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        Box::pin(async move {
            if command.trim().is_empty() {
                return Ok("no command provided".to_string());
            }

            if let Some(dir) = &cwd {
                if !std::path::Path::new(dir).is_dir() {
                    return Ok(format!("invalid working directory: {dir}"));
                }
            }

            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(&command);
            if let Some(dir) = &cwd {
                cmd.current_dir(dir);
            }
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            let child = cmd
                .spawn()
                .with_context(|| format!("failed to spawn command: {command}"))?;

            let output = tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                child.wait_with_output(),
            )
            .await
            .with_context(|| format!("command timed out after {timeout_secs}s: {command}"))??;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let status = output.status.code().unwrap_or(-1);

            Ok(format!(
                "exit_code: {status}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;
    use std::fs;

    #[test]
    fn name_returns_run_command() {
        assert_eq!(RunCommand.name(), "run_command");
    }

    #[test]
    fn description_not_empty() {
        assert!(!RunCommand.description().is_empty());
    }

    #[test]
    fn input_schema_requires_command() {
        let schema = RunCommand.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("command").is_some());
        assert!(schema["properties"].get("cwd").is_some());
        assert!(schema["properties"].get("timeout_secs").is_some());
        assert_eq!(schema["required"][0], "command");
    }

    #[tokio::test]
    async fn execute_runs_simple_command() {
        let result = RunCommand.execute(json!({"command": "echo hello"})).await.unwrap();
        assert!(result.contains("exit_code: 0"));
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn execute_captures_nonzero_exit_code() {
        let result = RunCommand.execute(json!({"command": "exit 1"})).await.unwrap();
        assert!(result.contains("exit_code: 1"));
    }

    #[tokio::test]
    async fn execute_respects_cwd() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("marker.txt"), "data").unwrap();

        let result = RunCommand
            .execute(json!({"command": "ls", "cwd": dir.path().to_str().unwrap()}))
            .await
            .unwrap();

        assert!(result.contains("marker.txt"));
    }

    #[tokio::test]
    async fn execute_empty_command_returns_message() {
        let result = RunCommand.execute(json!({"command": ""})).await.unwrap();
        assert_eq!(result, "no command provided");
    }

    #[tokio::test]
    async fn execute_invalid_cwd_returns_message() {
        let result = RunCommand
            .execute(json!({"command": "echo hi", "cwd": "/nonexistent/path/xyz"}))
            .await
            .unwrap();
        assert!(result.contains("invalid working directory"));
    }

    #[tokio::test]
    async fn execute_times_out_for_long_running_command() {
        let result = RunCommand
            .execute(json!({"command": "sleep 5", "timeout_secs": 1}))
            .await;
        assert!(result.is_err());
    }
}
