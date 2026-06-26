use std::future::Future;
use std::pin::Pin;
use std::path::{Path, Component};
use std::fs;

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub struct WriteFile;

impl super::Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file, creating or overwriting it. Rejects paths containing '..'."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write",
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file",
                }
            },
            "required": ["path", "content"]
        })
    }

    fn execute(&self, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let content = args
            .get("content")
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
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create parent directories for {path_str:?}"))?;
            }
            fs::write(path, &content)
                .with_context(|| format!("failed to write {path_str:?}"))?;
            Ok(format!("wrote {} bytes to {}", content.len(), path_str))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;

    #[test]
    fn name_returns_write_file() {
        assert_eq!(WriteFile.name(), "write_file");
    }

    #[test]
    fn description_not_empty() {
        assert!(!WriteFile.description().is_empty());
    }

    #[test]
    fn input_schema_requires_path_and_content() {
        let schema = WriteFile.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("content")));
    }

    #[tokio::test]
    async fn execute_missing_path_errors() {
        let result = WriteFile.execute(json!({"content": "data"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing required argument"));
    }

    #[tokio::test]
    async fn execute_path_traversal_errors() {
        let result = WriteFile
            .execute(json!({"path": "../escape.txt", "content": "x"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path traversal"));
    }

    #[tokio::test]
    async fn execute_writes_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("out.txt");

        let result = WriteFile
            .execute(json!({"path": file_path.to_str().unwrap(), "content": "hello"}))
            .await;
        assert!(result.is_ok());
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn execute_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("a/b/c/deep.txt");

        let result = WriteFile
            .execute(json!({"path": file_path.to_str().unwrap(), "content": "deep"}))
            .await;
        assert!(result.is_ok());
        assert!(file_path.exists());
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "deep");
    }

    #[tokio::test]
    async fn execute_returns_success_message() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("msg.txt");

        let result = WriteFile
            .execute(json!({"path": file_path.to_str().unwrap(), "content": "abc"}))
            .await
            .unwrap();
        assert!(result.contains("wrote 3 bytes to"));
    }
}
