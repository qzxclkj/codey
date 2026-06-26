//! Thin wrapper around the Ollama Chat API, plus the agent loop that
//! ties the tool-use protocol to the local tool registry.

use crate::app::Line;
use crate::tools::ToolRegistry;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use tokio::sync::mpsc;

pub const SYSTEM_PROMPT: &str = "\
Your name is Codey.  You are a senior software engineer embedded in a terminal application. You have \
access to file system tools (list_files, read_file, write_file). Use them \
whenever they would help answer the user's question, and explain findings \
concisely in plain language suitable for a terminal UI (avoid heavy markdown). \
You occasionally be conversational, but you should not respond with a question";

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait OllamaApi: Send + Sync {
    async fn messages(&self, messages: &[Value], tools: &[Value]) -> Result<Value>;
}

pub struct OllamaClient {
    model: String,
    host: String,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(ollama_host: String, ollama_model: String) -> Self {
        Self {
            model: ollama_model,
            host: ollama_host,
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl OllamaApi for OllamaClient {
    /// Send a single Messages API request and return the parsed JSON body.
    async fn messages(&self, messages: &[Value], tools: &[Value]) -> Result<Value> {
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "stream": false,
            "system": SYSTEM_PROMPT,
            "messages": messages,
            "tools": tools,
        });

        let resp = self
            .http
            .post(format!("{}{}", self.host, "/api/chat"))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("request to Ollama API failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("Ollama API error ({status}): {text}");
        }

        resp.json().await.context("failed to parse Ollama response")
    }
}

/// Messages sent from the background agent task back to the UI thread.
#[derive(Debug)]
pub enum AgentEvent {
    /// A line of output to append to the chat transcript.
    Line(Line),
    /// The turn finished; carries the updated conversation history so the
    /// UI thread can store it for the next turn.
    Done(Vec<Value>),
    /// The turn failed irrecoverably.
    Failed(String),
}

/// Global counter for generating unique tool call IDs.
static TOOL_CALL_COUNTER: AtomicI64 = AtomicI64::new(1);

/// Run one full "turn": send the conversation to Ollama, execute any tool
/// calls against the local registry, feed results back, and repeat until
/// Ollama responds without requesting further tools.
#[allow(clippy::too_many_arguments)]
pub async fn run_turn(
    mut history: Vec<Value>,
    tools: Vec<Value>,
    ollama: Arc<dyn OllamaApi>,
    registry: Arc<ToolRegistry>,
    tx: mpsc::UnboundedSender<AgentEvent>,
) {
    loop {
        let response = match ollama.messages(&history, &tools).await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(AgentEvent::Failed(e.to_string()));
                return;
            }
        };

        let message = match response.get("message") {
            Some(m) => m.clone(),
            None => {
                let _ = tx.send(AgentEvent::Failed(format!(
                    "unexpected response from Ollama: {response}"
                )));
                return;
            }
        };

        let text = message
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let tool_calls = message
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let mut assistant_msg = json!({
            "role": "assistant",
            "content": text,
        });
        if !tool_calls.is_empty() {
            assistant_msg["tool_calls"] = json!(tool_calls);
        }
        history.push(assistant_msg);

        if !text.trim().is_empty() {
            let _ = tx.send(AgentEvent::Line(Line::Assistant(text)));
        }

        if tool_calls.is_empty() {
            let _ = tx.send(AgentEvent::Done(history));
            return;
        }

        for tool_call in &tool_calls {
            let function = match tool_call.get("function") {
                Some(f) => f,
                None => {
                    let _ = tx.send(AgentEvent::Failed(format!(
                        "tool_call missing 'function': {tool_call}"
                    )));
                    return;
                }
            };

            let name = function
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown")
                .to_string();
            let input = function.get("arguments").cloned().unwrap_or(json!({}));

            let call_id = format!(
                "call_{}",
                TOOL_CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            );

            let _ = tx.send(AgentEvent::Line(Line::Tool(format!(
                "-> calling `{name}` with {input}"
            ))));

            let call_result = registry.execute(&name, input).await;

            let (content_text, is_error) = match call_result {
                Ok(text) => (text, false),
                Err(e) => (e.to_string(), true),
            };

            let _ = tx.send(AgentEvent::Line(Line::Tool(format!(
                "<- `{name}` {} ({} bytes)",
                if is_error { "errored" } else { "returned" },
                content_text.len()
            ))));

            history.push(json!({
                "role": "tool",
                "content": content_text,
                "tool_call_id": call_id,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use mockall::predicate::*;
    use serde_json::json;

    #[test]
    fn system_prompt_non_empty() {
        assert!(!SYSTEM_PROMPT.is_empty());
    }

    #[test]
    fn ollama_client_new_sets_fields() {
        let client = OllamaClient::new("http://test:11434".into(), "test-model".into());
        assert_eq!(client.model, "test-model");
        assert_eq!(client.host, "http://test:11434");
    }

    #[tokio::test]
    async fn run_turn_sends_text_and_done() {
        let mut mock = MockOllamaApi::new();
        mock.expect_messages()
            .with(always(), always())
            .returning(|_, _| {
                Ok(json!({
                    "message": {
                        "role": "assistant",
                        "content": "Hello, world!",
                    }
                }))
            });

        let registry = Arc::new(ToolRegistry::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(run_turn(
            vec![],
            vec![],
            Arc::new(mock),
            registry,
            tx,
        ));

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert_eq!(events.len(), 2);
        match &events[0] {
            AgentEvent::Line(Line::Assistant(text)) => assert_eq!(text, "Hello, world!"),
            other => panic!("expected Assistant line, got {other:?}"),
        }
        match &events[1] {
            AgentEvent::Done(history) => {
                assert_eq!(history.len(), 1);
                assert_eq!(history[0]["role"], "assistant");
                assert_eq!(history[0]["content"], "Hello, world!");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_turn_sends_failed_on_api_error() {
        let mut mock = MockOllamaApi::new();
        mock.expect_messages()
            .with(always(), always())
            .returning(|_, _| Err(anyhow::anyhow!("network error")));

        let registry = Arc::new(ToolRegistry::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(run_turn(
            vec![],
            vec![],
            Arc::new(mock),
            registry,
            tx,
        ));

        let event = rx.recv().await.unwrap();
        match event {
            AgentEvent::Failed(msg) => assert!(msg.contains("network error")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_turn_sends_failed_on_bad_response() {
        let mut mock = MockOllamaApi::new();
        mock.expect_messages()
            .with(always(), always())
            .returning(|_, _| Ok(json!({"no_message": "here"})));

        let registry = Arc::new(ToolRegistry::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(run_turn(
            vec![],
            vec![],
            Arc::new(mock),
            registry,
            tx,
        ));

        let event = rx.recv().await.unwrap();
        match event {
            AgentEvent::Failed(msg) => assert!(msg.contains("unexpected response")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_turn_executes_tool_and_loops() {
        static CALL_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

        let mut mock = MockOllamaApi::new();
        mock.expect_messages()
            .with(always(), always())
            .returning(|_, _| {
                let count = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if count == 0 {
                    // First call: respond with a tool_call
                    Ok(json!({
                        "message": {
                            "role": "assistant",
                            "content": "Let me list files.",
                            "tool_calls": [{
                                "function": {
                                    "name": "list_files",
                                    "arguments": {}
                                }
                            }]
                        }
                    }))
                } else {
                    // Second call: respond with text, no tool_calls
                    Ok(json!({
                        "message": {
                            "role": "assistant",
                            "content": "Here are the files.",
                        }
                    }))
                }
            });

        let registry = Arc::new(ToolRegistry::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(run_turn(
            vec![],
            vec![],
            Arc::new(mock),
            registry,
            tx,
        ));

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        // Expected: Assistant("Let me list files."), Tool("-> calling..."), Tool("<- ..."), Assistant("Here are the files."), Done(...)
        assert!(events.len() >= 4);
        match &events[0] {
            AgentEvent::Line(Line::Assistant(text)) => assert_eq!(text, "Let me list files."),
            other => panic!("expected Assistant, got {other:?}"),
        }
        match &events[1] {
            AgentEvent::Line(Line::Tool(msg)) => assert!(msg.contains("-> calling `list_files`")),
            other => panic!("expected Tool, got {other:?}"),
        }
        match &events[2] {
            AgentEvent::Line(Line::Tool(msg)) => assert!(msg.contains("<- `list_files`")),
            other => panic!("expected Tool, got {other:?}"),
        }
        match &events[3] {
            AgentEvent::Line(Line::Assistant(text)) => assert_eq!(text, "Here are the files."),
            other => panic!("expected Assistant, got {other:?}"),
        }
        match events.last().unwrap() {
            AgentEvent::Done(history) => {
                assert_eq!(history.len(), 3); // assistant + tool_result + final assistant
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_turn_sends_failed_on_bad_tool_call() {
        let mut mock = MockOllamaApi::new();
        mock.expect_messages()
            .with(always(), always())
            .returning(|_, _| {
                Ok(json!({
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "no_function": "here"
                        }]
                    }
                }))
            });

        let registry = Arc::new(ToolRegistry::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(run_turn(
            vec![],
            vec![],
            Arc::new(mock),
            registry,
            tx,
        ));

        let event = rx.recv().await.unwrap();
        match event {
            AgentEvent::Failed(msg) => assert!(msg.contains("tool_call missing 'function'")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
