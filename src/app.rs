use crate::agent::{self, AgentEvent, OllamaClient};
// use crate::mcp::{McpClient, McpTool};
use anyhow::Result;
use crossterm::event::KeyCode;
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// One entry in the chat transcript.
#[derive(Clone)]
pub enum Line {
    User(String),
    Assistant(String),
    /// Status / tool-call information, rendered dimmed.
    Tool(String),
    Error(String),
    System(String),
}

pub struct App {
    pub input: String,
    pub transcript: Vec<Line>,
    pub scroll: u16,
    pub processing: bool,
    pub should_quit: bool,
    pub tool_count: usize,

    /// Conversation history in Ollama Messages API format. Persists
    /// across turns so the agent has memory of the conversation.
    history: Vec<Value>,
    /// Tool definitions in Ollama/OpenAI `tools` format, derived once from
    /// the MCP server's `tools/list` response.
    tools_json: Vec<Value>,

    // mcp: Arc<Mutex<McpClient>>,
    ollama: Arc<OllamaClient>,

    agent_tx: mpsc::UnboundedSender<AgentEvent>,
    pub agent_rx: mpsc::UnboundedReceiver<AgentEvent>,
}

impl App {
    /// Spawn the MCP server, perform the handshake, fetch its tool list,
    /// and set up the Ollama client. Reads configuration from env vars
    /// (a `.env` file in the working directory is loaded automatically by
    /// `main`):
    ///
    /// - `OLLAMA_HOST`  (default: "http://localhost:11434")
    /// - `OLLAMA_MODEL` (default: "qwen2.5", must be a model that supports
    ///                   tool calling -- e.g. qwen2.5, llama3.1+, mistral-nemo)
    /// - `MCP_SERVER_COMMAND` (default: "npx")
    /// - `MCP_SERVER_ARGS`    (default: "-y @awesome-ai/elasticsearch-mcp",
    ///                         space-separated)
    /// Any `ES_HOST`, `ES_API_KEY` (or `ES_USERNAME`/`ES_PASSWORD`) env vars
    /// used by the Elasticsearch MCP server should already be set (shell or
    /// `.env`) -- they are inherited by the spawned process automatically.
    pub async fn new() -> Result<Self> {
        let ollama_host =
            env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
        let ollama_model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5".to_string());

        let command = env::var("MCP_SERVER_COMMAND").unwrap_or_else(|_| "npx".to_string());
        let args: Vec<String> = env::var("MCP_SERVER_ARGS")
            .unwrap_or_else(|_| "-y @awesome-ai/elasticsearch-mcp".to_string())
            .split_whitespace()
            .map(String::from)
            .collect();

        // let mut mcp = McpClient::spawn(&command, &args, &[]).await?;
        // mcp.initialize().await?;
        // let tools = mcp.list_tools().await?;
        let tools = vec![];
        // let tools_json = tools.iter().map(mcp_tool_to_ollama_tool).collect();
        let tools_json = tools.clone();
        // let tool_count = tools.len();
        let tool_count = tools.len();

        let (agent_tx, agent_rx) = mpsc::unbounded_channel();

        let mut transcript = vec![Line::System(format!(
            "Connected to MCP server `{command} {}` -- {tool_count} tool(s) available.",
            args.join(" ")
        ))];
        // if tool_count > 0 {
        //     // let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        //     transcript.push(Line::System(format!("Tools: {}", names.join(", "))));
        // }
        // transcript.push(Line::System(format!(
        //     "Using Ollama model `{ollama_model}` at {ollama_host}."
        // )));
        // transcript.push(Line::System(
        //     "Type a message and press Enter. Esc to quit.".to_string(),
        // ));

        Ok(Self {
            input: String::new(),
            transcript,
            scroll: 0,
            processing: false,
            should_quit: false,
            tool_count,
            history: vec![],
            tools_json,
            // mcp: Arc::new(Mutex::new(mcp)),
            ollama: Arc::new(OllamaClient::new(ollama_host, ollama_model)),
            agent_tx,
            agent_rx,
        })
    }

    /// Handle a key press. Returns `Ok(())`; sets `should_quit` when the
    /// app should exit.
    pub fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            _ => {}
        }
    }

    fn submit(&mut self) {
        if self.processing {
            return;
        }
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.input.clear();
        self.transcript.push(Line::User(text.clone()));

        self.history.push(json!({
            "role": "user",
            "content": text,
        }));

        self.processing = true;
        let history = self.history.clone();
        let tools_json = self.tools_json.clone();
        let ollama = self.ollama.clone();
        // let mcp = self.mcp.clone();
        let tx = self.agent_tx.clone();

        // tokio::spawn(agent::run_turn(history, tools_json, ollama, mcp, tx));
        tokio::spawn(agent::run_turn(history, tools_json, ollama, tx));
    }

    /// Apply an event coming back from the background agent task.
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Line(line) => self.transcript.push(line),
            AgentEvent::Done(history) => {
                self.history = history;
                self.processing = false;
            }
            AgentEvent::Failed(err) => {
                self.transcript.push(Line::Error(err));
                self.processing = false;
            }
        }
        // Auto-scroll to the bottom whenever new content arrives.
        self.scroll = u16::MAX;
    }
}

/*
/// Convert an MCP tool definition into the shape Ollama's `tools` parameter
/// expects: an OpenAI-style `{type: "function", function: {name,
/// description, parameters}}` wrapper around the tool's JSON Schema.
// fn mcp_tool_to_ollama_tool(tool: &McpTool) -> Value {
//     json!({
//         "type": "function",
//         "function": {
//             "name": tool.name,
//             "description": tool.description,
//             "parameters": tool.input_schema,
//         }
//     })
// }
*/