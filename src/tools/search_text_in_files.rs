use std::future::Future;
use std::pin::Pin;
use std::path::Path;
use std::fs;

use anyhow::{Context, Result};
use serde_json::Value;

const MAX_DEPTH: usize = 10;

pub struct SearchTextInFiles;

impl super::Tool for SearchTextInFiles {
    fn name(&self) -> &str {
        "search_text_in_files"
    }

    fn description(&self) -> &str {
        "Recursively search for a text query in all files under a directory (default: current working directory), \
         limiting recursion to a depth of 10. Optionally case-insensitive. Returns matching lines formatted as \
         `path:line_number: line_content`."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Text to search for in file contents"
                },
                "root": {
                    "type": "string",
                    "description": "Root directory to search (default: current working directory)"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Whether the search should ignore case (default: false)"
                }
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let root = args
            .get("root")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Box::pin(async move {
            if query.is_empty() {
                return Ok(String::new());
            }

            let mut matches = Vec::new();
            search_dir(Path::new(&root), &query, case_insensitive, 0, &mut matches)
                .with_context(|| format!("failed to search files under {root:?}"))?;
            Ok(matches.join("\n"))
        })
    }
}

fn search_dir(
    dir: &Path,
    query: &str,
    case_insensitive: bool,
    depth: usize,
    out: &mut Vec<String>,
) -> std::io::Result<()> {
    if depth > MAX_DEPTH || !dir.is_dir() {
        return Ok(());
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()), // skip unreadable directories
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            search_dir(&path, query, case_insensitive, depth + 1, out)?;
        } else if path.is_file() {
            search_file(&path, query, case_insensitive, out);
        }
    }

    Ok(())
}

fn search_file(path: &Path, query: &str, case_insensitive: bool, out: &mut Vec<String>) {
    // Skip files that can't be read as UTF-8 text (e.g. binaries)
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let needle = if case_insensitive {
        query.to_lowercase()
    } else {
        query.to_string()
    };

    for (idx, line) in content.lines().enumerate() {
        let haystack = if case_insensitive {
            line.to_lowercase()
        } else {
            line.to_string()
        };

        if haystack.contains(&needle) {
            out.push(format!("{}:{}: {}", path.to_string_lossy(), idx + 1, line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;
    use std::fs;

    #[test]
    fn name_returns_search_text_in_files() {
        assert_eq!(SearchTextInFiles.name(), "search_text_in_files");
    }

    #[test]
    fn description_not_empty() {
        assert!(!SearchTextInFiles.description().is_empty());
    }

    #[test]
    fn input_schema_requires_query() {
        let schema = SearchTextInFiles.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("query").is_some());
        assert!(schema["properties"].get("root").is_some());
        assert_eq!(schema["required"][0], "query");
    }

    #[tokio::test]
    async fn execute_finds_matches() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello world\nfoo bar").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("b.txt"), "another hello here").unwrap();

        let result = SearchTextInFiles
            .execute(json!({"query": "hello", "root": dir.path().to_str().unwrap()}))
            .await
            .unwrap();

        assert!(result.contains("a.txt:1: hello world"));
        assert!(result.contains("b.txt:1: another hello here"));
        assert!(!result.contains("foo bar"));
    }

    #[tokio::test]
    async fn execute_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "HELLO world").unwrap();

        let result = SearchTextInFiles
            .execute(json!({
                "query": "hello",
                "root": dir.path().to_str().unwrap(),
                "case_insensitive": true
            }))
            .await
            .unwrap();

        assert!(result.contains("HELLO world"));
    }

    #[tokio::test]
    async fn execute_empty_query_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let result = SearchTextInFiles
            .execute(json!({"query": "", "root": dir.path().to_str().unwrap()}))
            .await
            .unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn search_dir_respects_max_depth() {
        let dir = tempfile::tempdir().unwrap();
        let mut current = dir.path().to_path_buf();
        for i in 0..15 {
            current = current.join(format!("d{i}"));
            fs::create_dir(&current).unwrap();
        }
        fs::write(current.join("deep.txt"), "needle").unwrap();

        let mut out = Vec::new();
        search_dir(dir.path(), "needle", false, 0, &mut out).unwrap();

        // File is beyond MAX_DEPTH, so it should not be found.
        assert!(out.is_empty());
    }
}
