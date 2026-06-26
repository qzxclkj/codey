use std::future::Future;
use std::pin::Pin;
use std::path::{Path, Component};
use std::fs;

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub struct ReadFile;

impl super::Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path. Rejects paths containing '..'."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read",
                }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Box::pin(async move {
            if path_str.is_empty() {
                bail!("missing required argument: 'path'");
            }
            let path = Path::new(&path_str);
            if path.components().any(|c| c == Component::ParentDir) {
                bail!("path traversal detected: '..' is not allowed");
            }
            let content = fs::read_to_string(path)
                .with_context(|| format!("failed to read {path_str:?}"))?;
            Ok(content)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;

    #[test]
    fn name_returns_read_file() {
        assert_eq!(ReadFile.name(), "read_file");
    }

    #[test]
    fn description_not_empty() {
        assert!(!ReadFile.description().is_empty());
    }

    #[test]
    fn input_schema_has_path_required() {
        let schema = ReadFile.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["required"].as_array().unwrap().contains(&json!("path")));
    }

    #[tokio::test]
    async fn execute_missing_path_errors() {
        let result = ReadFile.execute(json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing required argument"));
    }

    #[tokio::test]
    async fn execute_path_traversal_errors() {
        let result = ReadFile.execute(json!({"path": "../etc/passwd"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path traversal"));
    }

    #[tokio::test]
    async fn execute_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let result = ReadFile
            .execute(json!({"path": file_path.to_str().unwrap()}))
            .await;
        assert_eq!(result.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn execute_file_not_found() {
        let result = ReadFile.execute(json!({"path": "/tmp/nonexistent_12345"})).await;
        assert!(result.is_err());
    }
}
