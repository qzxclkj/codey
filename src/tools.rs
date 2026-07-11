pub mod list_files;
pub mod read_file;
pub mod write_file;
pub mod search_text_in_files;
pub mod run_command;

use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn execute(&self, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut reg = Self { tools: Vec::new() };
        reg.register(Box::new(list_files::ListFiles));
        reg.register(Box::new(read_file::ReadFile));
        reg.register(Box::new(write_file::WriteFile));
        reg.register(Box::new(search_text_in_files::SearchTextInFiles));
        reg.register(Box::new(run_command::RunCommand));
        reg
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|b| b.as_ref())
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        let tool = self
            .get(name)
            .with_context(|| format!("unknown tool: {name}"))?;
        tool.execute(args).await
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn to_ai_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name(),
                        "description": tool.description(),
                        "parameters": tool.input_schema(),
                    }
                })
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_contains_three_tools() {
        let reg = ToolRegistry::new();
        assert_eq!(reg.len(), 5);
    }

    #[test]
    fn register_increases_len() {
        let mut reg = ToolRegistry::new();
        assert_eq!(reg.len(), 5);
        reg.register(Box::new(list_files::ListFiles));
        assert_eq!(reg.len(), 6);
    }

    #[test]
    fn get_returns_some_for_known_tools() {
        let reg = ToolRegistry::new();
        assert!(reg.get("read_file").is_some());
        assert!(reg.get("write_file").is_some());
        assert!(reg.get("list_files").is_some());
    }

    #[test]
    fn get_returns_none_for_unknown() {
        let reg = ToolRegistry::new();
        assert!(reg.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn execute_known_tool_succeeds() {
        let reg = ToolRegistry::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "data").unwrap();

        let result = reg
            .execute("read_file", json!({"path": file_path.to_str().unwrap()}))
            .await;
        assert_eq!(result.unwrap(), "data");
    }

    #[tokio::test]
    async fn execute_unknown_tool_errors() {
        let reg = ToolRegistry::new();
        let result = reg.execute("no_such_tool", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown tool"));
    }

    #[test]
    fn to_ai_tools_format() {
        let reg = ToolRegistry::new();
        let tools = reg.to_ai_tools();
        assert_eq!(tools.len(), 5);

        for tool_val in &tools {
            assert_eq!(tool_val["type"], "function");
            let function = &tool_val["function"];
            assert!(function["name"].as_str().is_some());
            assert!(function["description"].as_str().is_some());
            assert!(function["parameters"]["type"].as_str().is_some());
        }

        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"list_files"));
        assert!(names.contains(&"search_text_in_files"));
        assert!(names.contains(&"run_command"));
    }
}
