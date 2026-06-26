use crate::agent::{self, OllamaApi, AgentEvent};
use crate::tools::ToolRegistry;
use anyhow::Result;
use crossterm::event::KeyCode;
use serde_json::{Value, json};
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc;

/// One entry in the chat transcript.
#[derive(Clone, Debug)]
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
    pub history: Vec<Value>,
    tools_json: Vec<Value>,

    registry: Arc<ToolRegistry>,
    ollama: Arc<dyn OllamaApi>,

    agent_tx: mpsc::UnboundedSender<AgentEvent>,
    pub agent_rx: mpsc::UnboundedReceiver<AgentEvent>,
}

impl App {
    /// Public async constructor used by main.rs.  Reads configuration from
    /// env vars (a `.env` file is loaded automatically by `main`).
    ///
    /// - `OLLAMA_HOST`  (default: "http://localhost:11434")
    /// - `OLLAMA_MODEL` (default: "qwen2.5")
    pub async fn new() -> Result<Self> {
        let ollama_host =
            env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
        let ollama_model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5".to_string());

        let registry = Arc::new(ToolRegistry::new());
        let ollama = Arc::new(agent::OllamaClient::new(ollama_host, ollama_model));
        let (agent_tx, agent_rx) = mpsc::unbounded_channel();

        Ok(Self::new_with(registry, ollama as Arc<dyn OllamaApi>, agent_tx, agent_rx))
    }

    /// Injectable constructor for tests.  Takes an already-built registry,
    /// API client, and channel pair.
    pub fn new_with(
        registry: Arc<ToolRegistry>,
        ollama: Arc<dyn OllamaApi>,
        agent_tx: mpsc::UnboundedSender<AgentEvent>,
        agent_rx: mpsc::UnboundedReceiver<AgentEvent>,
    ) -> Self {
        let tool_count = registry.len();
        let tools_json = registry.to_ai_tools();

        let mut transcript = vec![];
        transcript.push(Line::System(format!(
            "Registered {tool_count} local tool(s)"
        )));
        transcript.push(Line::System("Using Ollama model ...".to_string()));

        Self {
            input: String::new(),
            transcript,
            scroll: 0,
            processing: false,
            should_quit: false,
            tool_count,
            history: vec![],
            tools_json,
            registry,
            ollama,
            agent_tx,
            agent_rx,
        }
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

    pub fn submit(&mut self) {
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
        let registry = self.registry.clone();
        let tx = self.agent_tx.clone();

        tokio::spawn(agent::run_turn(history, tools_json, ollama, registry, tx));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::OllamaApi;
    use crate::tools::ToolRegistry;
    use crossterm::event::KeyCode;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    struct NoopOllama;
    #[async_trait::async_trait]
    impl OllamaApi for NoopOllama {
        async fn messages(&self, _: &[Value], _: &[Value]) -> anyhow::Result<Value> {
            Ok(json!({}))
        }
    }

    fn make_app() -> App {
        let registry = Arc::new(ToolRegistry::new());
        let ollama = Arc::new(NoopOllama);
        let (tx, rx) = mpsc::unbounded_channel();
        App::new_with(registry, ollama as Arc<dyn OllamaApi>, tx, rx)
    }

    #[test]
    fn new_with_sets_initial_state() {
        let app = make_app();
        assert!(app.input.is_empty());
        assert!(!app.transcript.is_empty());
        assert_eq!(app.scroll, 0);
        assert!(!app.processing);
        assert!(!app.should_quit);
        assert_eq!(app.tool_count, 3);
        assert!(app.history.is_empty());
    }

    #[test]
    fn esc_sets_should_quit() {
        let mut app = make_app();
        app.handle_key(KeyCode::Esc);
        assert!(app.should_quit);
    }

    #[test]
    fn backspace_pops_char() {
        let mut app = make_app();
        app.input.push_str("hello");
        app.handle_key(KeyCode::Backspace);
        assert_eq!(app.input, "hell");
    }

    #[test]
    fn char_appends_input() {
        let mut app = make_app();
        app.handle_key(KeyCode::Char('x'));
        assert_eq!(app.input, "x");
    }

    #[test]
    fn up_adjusts_scroll() {
        let mut app = make_app();
        app.scroll = 5;
        app.handle_key(KeyCode::Up);
        assert_eq!(app.scroll, 4);
    }

    #[test]
    fn down_adjusts_scroll() {
        let mut app = make_app();
        app.scroll = 5;
        app.handle_key(KeyCode::Down);
        assert_eq!(app.scroll, 6);
    }

    #[test]
    fn up_saturates_at_zero() {
        let mut app = make_app();
        app.scroll = 0;
        app.handle_key(KeyCode::Up);
        assert_eq!(app.scroll, 0);
    }

    #[tokio::test]
    async fn submit_noop_when_processing() {
        let mut app = make_app();
        app.processing = true;
        app.input.push_str("hello");
        app.submit();
        assert_eq!(app.input, "hello");
        assert!(app.history.is_empty());
    }

    #[tokio::test]
    async fn submit_noop_when_empty() {
        let mut app = make_app();
        app.input.clear();
        app.submit();
        assert!(!app.processing);
    }

    #[tokio::test]
    async fn submit_adds_user_line() {
        let mut app = make_app();
        app.input.push_str("hi there");
        app.submit();

        assert!(app.input.is_empty());
        assert!(app.processing);
        assert_eq!(app.history.len(), 1);
        assert_eq!(app.history[0]["role"], "user");
        assert_eq!(app.history[0]["content"], "hi there");

        let user_line = app
            .transcript
            .iter()
            .find(|l| matches!(l, Line::User(_)))
            .unwrap();
        match user_line {
            Line::User(text) => assert_eq!(text, "hi there"),
            _ => panic!("expected User line"),
        }
    }

    #[tokio::test]
    async fn submit_trims_input() {
        let mut app = make_app();
        app.input.push_str("  spaced  ");
        app.submit();
        assert_eq!(app.history[0]["content"], "spaced");
    }

    #[test]
    fn handle_agent_event_line_appends() {
        let mut app = make_app();
        let before = app.transcript.len();
        app.handle_agent_event(AgentEvent::Line(Line::Assistant("test".into())));
        assert_eq!(app.transcript.len(), before + 1);
        match app.transcript.last().unwrap() {
            Line::Assistant(text) => assert_eq!(text, "test"),
            _ => panic!("expected Assistant"),
        }
        assert_eq!(app.scroll, u16::MAX);
    }

    #[test]
    fn handle_agent_event_done_updates_history() {
        let mut app = make_app();
        app.processing = true;
        let new_history = vec![json!({"role": "assistant", "content": "done"})];
        app.handle_agent_event(AgentEvent::Done(new_history.clone()));
        assert!(!app.processing);
        assert_eq!(app.history, new_history);
    }

    #[test]
    fn handle_agent_event_failed_appends_error() {
        let mut app = make_app();
        app.processing = true;
        app.handle_agent_event(AgentEvent::Failed("boom".into()));
        assert!(!app.processing);
        match app.transcript.last().unwrap() {
            Line::Error(text) => assert_eq!(text, "boom"),
            _ => panic!("expected Error"),
        }
    }
}
