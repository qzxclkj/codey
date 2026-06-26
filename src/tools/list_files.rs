use std::future::Future;
use std::pin::Pin;
use std::path::Path;
use std::fs;

use anyhow::{Context, Result};
use serde_json::Value;

pub struct ListFiles;

impl super::Tool for ListFiles {
    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "Recursively list all files under a directory. Takes an optional `root` argument (default: \".\"). Returns one file path per line."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "root": {
                    "type": "string",
                    "description": "Root directory to list (default: current working directory)",
                }
            }
        })
    }

    fn execute(&self, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>> {
        let root = args
            .get("root")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        Box::pin(async move {
            let mut files = Vec::new();
            collect_files(Path::new(&root), &mut files)
                .with_context(|| format!("failed to list files under {root:?}"))?;
            Ok(files.join("\n"))
        })
    }
}

pub fn collect_files(dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, out)?;
            } else {
                out.push(path.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;
    use std::fs;

    #[test]
    fn name_returns_list_files() {
        assert_eq!(ListFiles.name(), "list_files");
    }

    #[test]
    fn description_not_empty() {
        assert!(!ListFiles.description().is_empty());
    }

    #[test]
    fn input_schema_has_optional_root() {
        let schema = ListFiles.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("root").is_some());
        assert!(schema.get("required").is_none());
    }

    #[tokio::test]
    async fn execute_with_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "").unwrap();
        fs::write(dir.path().join("b.txt"), "").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("c.txt"), "").unwrap();

        let result = ListFiles
            .execute(json!({"root": dir.path().to_str().unwrap()}))
            .await
            .unwrap();

        assert!(result.contains("a.txt"));
        assert!(result.contains("b.txt"));
        assert!(result.contains("c.txt"));
    }

    #[tokio::test]
    async fn execute_with_nonexistent_root_returns_empty() {
        let result = ListFiles
            .execute(json!({"root": "/tmp/nonexistent_dir_98765"}))
            .await
            .unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn collect_files_recursive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("top.txt"), "").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("nested.txt"), "").unwrap();

        let mut files = Vec::new();
        collect_files(dir.path(), &mut files).unwrap();

        assert!(files.iter().any(|f| f.ends_with("top.txt")));
        assert!(files.iter().any(|f| f.ends_with("nested.txt")));
    }

    #[test]
    fn collect_files_skips_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("emptydir")).unwrap();

        let mut files = Vec::new();
        collect_files(dir.path(), &mut files).unwrap();

        assert!(files.iter().all(|f| !f.ends_with("emptydir")));
    }

    #[test]
    fn collect_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        collect_files(dir.path(), &mut files).unwrap();
        assert!(files.is_empty());
    }
}
