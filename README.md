# codey - ai coding agent

A small `ratatui` terminal chat UI that acts as an AI agent: user messages
go to an LLM (via Ollama API), and the LLM can call tools.

```
┌ Codey AI Agent · 8 tool(s) ·  ready  ────────────────────────┐
├ Conversation ────────────────────────────────────────────────┤
│ you                                                          │
│   Hello! I want to create a rust web server project.         │
│                                                              │
│ agent                                                        │
│   Sure, I can do that for you....                            │
└──────────────────────────────────────────────────────────────┘
┌ Message (Enter to send, Esc to quit) ────────────────────────┐
│|▏                                                            │
└──────────────────────────────────────────────────────────────┘
```

## How it fits together

- **`src/agent.rs`** -- wraps the Ollama Chat API and implements the
  agent loop: send conversation -> if the LLM responds with `tool_use`,
  execute it, append a `tool_result`, and send again -- repeating
  until the LLM replies with plain text.
- **`src/app.rs`** -- application state (transcript, input, conversation
  history, processing flag) and key handling. Each user message spawns a
  `tokio` task running the agent loop; results stream back over an
  `mpsc` channel so the UI never blocks.
- **`src/ui.rs`** -- the `ratatui` layout: header (status + tool count),
  scrollable conversation pane, input box.
- **`src/main.rs`** -- terminal setup/teardown and the main event loop,
  using `crossterm::event::EventStream` selected against the agent's
  `mpsc` channel.

