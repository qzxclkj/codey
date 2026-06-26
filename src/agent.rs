//! Thin wrapper around the Ollama Chat API, plus the agent loop that
//! ties the tool-use protocol to the MCP client.

use crate::app::Line;
// use crate::mcp::McpClient;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub const SYSTEM_PROMPT: &str = "\
You are an SRE assistant embedded in a terminal application. You have \
access to tools that query an Elasticsearch cluster (indices, documents, \
search, mappings, cluster health, etc). Use the tools whenever they would help answer the \
user's question, and explain findings concisely in plain language suitable \
for a terminal UI (avoid heavy markdown).";

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

    /// Send a single Messages API request and return the parsed JSON body.
    pub async fn messages(&self, messages: &[Value], tools: &[Value]) -> Result<Value> {
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
/// calls against the MCP server, feed results back, and repeat until Ollama
/// responds without requesting further tools.
pub async fn run_turn(
    mut history: Vec<Value>,
    tools: Vec<Value>,
    ollama: Arc<OllamaClient>,
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

        // Ollama wraps the response in a "message" object:
        //   { "message": { "role": "assistant", "content": "...",
        //                   "tool_calls": [{"function": {"name":"...","arguments":{...}}}] } }
        let message = match response.get("message") {
            Some(m) => m.clone(),
            None => {
                let _ = tx.send(AgentEvent::Failed(format!(
                    "unexpected response from Ollama: {response}"
                )));
                return;
            }
        };

        // Extract text content (Ollama returns it as a plain string).
        let text = message
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        // Extract tool calls (Ollama OpenAI-compatible format).
        let tool_calls = message
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        // Build and store the assistant message in Ollama/OpenAI format.
        let mut assistant_msg = json!({
            "role": "assistant",
            "content": text,
        });
        if !tool_calls.is_empty() {
            assistant_msg["tool_calls"] = json!(tool_calls);
        }
        history.push(assistant_msg);

        // Emit any text response from the assistant.
        if !text.trim().is_empty() {
            let _ = tx.send(AgentEvent::Line(Line::Assistant(text)));
        }

        if tool_calls.is_empty() {
            // No more tools requested -- this turn is complete.
            let _ = tx.send(AgentEvent::Done(history));
            return;
        }

        // Execute each requested tool call against the MCP server and
        // collect tool result messages for the next request.
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

            // Generate a unique ID for this tool call so we can pair the
            // result back to it in the Ollama/OpenAI format.
            let call_id = format!(
                "call_{}",
                TOOL_CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            );

            let _ = tx.send(AgentEvent::Line(Line::Tool(format!(
                "-> calling `{name}` with {input}"
            ))));

            // let call_result = {
            //     let mut guard = mcp.lock().await;
            //     guard.call_tool(&name, input).await
            // };
            //
            // let (content_text, is_error) = match call_result {
            //     Ok(text) => (text, false),
            //     Err(e) => (e.to_string(), true),
            // };

            // let _ = tx.send(AgentEvent::Line(Line::Tool(format!(
            //     "<- `{name}` {} ({} bytes)",
            //     if is_error { "errored" } else { "returned" },
            //     content_text.len()
            // ))));
            //
            // // Use Ollama/OpenAI tool result format:
            // //   { "role": "tool", "content": "...", "tool_call_id": "call_..." }
            // history.push(json!({
            //     "role": "tool",
            //     "content": content_text,
            //     "tool_call_id": call_id,
            // }));
        }

        // Loop again: send the updated history (including tool results)
        // back to Ollama.
    }
}
