//! TencentDB-Agent-Memory MCP Server (Rust)
//!
//! Wraps the TDAI Hermes Gateway HTTP API as an MCP server,
//! enabling Claude Code to use TencentDB memory capabilities
//! via the Model Context Protocol.
//!
//! Architecture:
//!   Claude Code <--MCP (stdio)--> this binary <--HTTP--> TDAI Gateway (:8420)

use std::env;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    schemars, tool, tool_router, tool_handler,
    transport::io::stdio,
};
use serde::Deserialize;
use serde_json::{json, Value};

// ============================
// Configuration
// ============================

#[derive(Clone)]
struct Config {
    gateway_url: String,
    gateway_api_key: String,
    default_session_key: String,
}

impl Config {
    fn from_env() -> Self {
        Self {
            gateway_url: env::var("TDAI_GATEWAY_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8420".into())
                .trim_end_matches('/')
                .to_string(),
            gateway_api_key: env::var("TDAI_GATEWAY_API_KEY").unwrap_or_default(),
            default_session_key: env::var("TDAI_SESSION_KEY")
                .unwrap_or_else(|_| "claude-code".into()),
        }
    }
}

// ============================
// Tool Parameter Structs
// ============================

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MemorySearchParams {
    /// Search query - keywords or natural language question
    query: String,
    /// Max results to return (default: 5)
    #[serde(default)]
    limit: Option<u32>,
    /// Filter by memory type (e.g. 'preference', 'fact', 'instruction')
    #[serde(default, rename = "type")]
    memory_type: Option<String>,
    /// Filter by scene name (e.g. 'coding-style', 'project-setup')
    #[serde(default)]
    scene: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConversationSearchParams {
    /// Search query - keywords or natural language
    query: String,
    /// Max results (default: 5)
    #[serde(default)]
    limit: Option<u32>,
    /// Filter by session key (optional)
    #[serde(default)]
    session_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RecallParams {
    /// The user's current query or topic to recall context for
    query: String,
    /// Session key (default: 'claude-code')
    #[serde(default)]
    session_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CaptureParams {
    /// The user's message
    user_content: String,
    /// The assistant's response
    assistant_content: String,
    /// Session key (default: 'claude-code')
    #[serde(default)]
    session_key: Option<String>,
    /// Sub-session ID (optional)
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SessionEndParams {
    /// Session key to end (default: 'claude-code')
    #[serde(default)]
    session_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SeedParams {
    /// JSON string of conversation data
    data: String,
    /// Fallback session key when input sessions lack one
    #[serde(default)]
    session_key: Option<String>,
    /// Auto-fill missing timestamps (default: true)
    #[serde(default)]
    auto_fill_timestamps: Option<bool>,
}

// ============================
// Server
// ============================

#[derive(Clone)]
struct TdaiServer {
    config: Config,
    client: reqwest::Client,
}

impl TdaiServer {
    fn new() -> Self {
        Self {
            config: Config::from_env(),
            client: reqwest::Client::new(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}{}", self.config.gateway_url, path);
        let mut req = self.client.get(&url);
        if !self.config.gateway_api_key.is_empty() {
            req = req.bearer_auth(&self.config.gateway_api_key);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let text = resp.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|_| text)
    }

    async fn post(&self, path: &str, body: Value) -> Result<(bool, Value), String> {
        let url = format!("{}{}", self.config.gateway_url, path);
        let mut req = self.client.post(&url)
            .header("Content-Type", "application/json")
            .json(&body);
        if !self.config.gateway_api_key.is_empty() {
            req = req.bearer_auth(&self.config.gateway_api_key);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let ok = resp.status().is_success();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        let data: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok((ok, data))
    }

    fn resolve_session_key(&self, key: Option<String>) -> String {
        key.filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.config.default_session_key.clone())
    }
}

// ============================
// Tool Implementations
// ============================

#[tool_router]
impl TdaiServer {
    #[tool(description = "Check the TDAI Gateway health status. Returns store availability, uptime, and version.")]
    async fn tdai_health(&self) -> String {
        match self.get("/health").await {
            Ok(data) => format!(
                "Status: {}\nVersion: {}\nUptime: {}s\nVectorStore: {}\nEmbeddingService: {}\nGateway URL: {}",
                data["status"].as_str().unwrap_or("unknown"),
                data["version"].as_str().unwrap_or("unknown"),
                data["uptime"].as_u64().unwrap_or(0),
                if data["stores"]["vectorStore"].as_bool().unwrap_or(false) { "OK" } else { "unavailable" },
                if data["stores"]["embeddingService"].as_bool().unwrap_or(false) { "OK" } else { "unavailable" },
                self.config.gateway_url,
            ),
            Err(e) => format!("Cannot connect to Gateway at {}: {}", self.config.gateway_url, e),
        }
    }

    #[tool(description = "Search L1 structured memories (atomic facts extracted from past conversations). Supports hybrid search (BM25 keyword + vector embedding + RRF fusion).")]
    async fn tdai_memory_search(&self, Parameters(p): Parameters<MemorySearchParams>) -> String {
        let mut body = json!({ "query": p.query, "limit": p.limit.unwrap_or(5) });
        if let Some(t) = p.memory_type {
            body["type"] = json!(t);
        }
        if let Some(s) = p.scene {
            body["scene"] = json!(s);
        }

        match self.post("/search/memories", body).await {
            Ok((true, data)) => format!(
                "[Strategy: {} | Total: {}]\n\n{}",
                data["strategy"].as_str().unwrap_or("unknown"),
                data["total"].as_u64().unwrap_or(0),
                data["results"].as_str().unwrap_or(""),
            ),
            Ok((false, data)) => format!("Search error: {}", data),
            Err(e) => format!("Memory search failed: {}", e),
        }
    }

    #[tool(description = "Search L0 raw conversation history (original user/assistant messages). Use this to find exact past dialogue.")]
    async fn tdai_conversation_search(&self, Parameters(p): Parameters<ConversationSearchParams>) -> String {
        let mut body = json!({ "query": p.query, "limit": p.limit.unwrap_or(5) });
        if let Some(sk) = p.session_key {
            body["session_key"] = json!(sk);
        }

        match self.post("/search/conversations", body).await {
            Ok((true, data)) => format!(
                "[Total: {}]\n\n{}",
                data["total"].as_u64().unwrap_or(0),
                data["results"].as_str().unwrap_or(""),
            ),
            Ok((false, data)) => format!("Search error: {}", data),
            Err(e) => format!("Conversation search failed: {}", e),
        }
    }

    #[tool(description = "Recall relevant memories for a given query/context. Returns L1 memories, L3 persona context, and scene navigation hints optimized for LLM context injection.")]
    async fn tdai_recall(&self, Parameters(p): Parameters<RecallParams>) -> String {
        let sk = self.resolve_session_key(p.session_key);
        let body = json!({ "query": p.query, "session_key": sk });

        match self.post("/recall", body).await {
            Ok((true, data)) => {
                let ctx = data["context"].as_str().unwrap_or("");
                if ctx.trim().is_empty() {
                    return "No relevant memories found for this query.".into();
                }
                format!(
                    "[Strategy: {} | Memories: {}]\n\n{}",
                    data["strategy"].as_str().unwrap_or("unknown"),
                    data["memory_count"].as_u64().unwrap_or(0),
                    ctx,
                )
            }
            Ok((false, data)) => format!("Recall error: {}", data),
            Err(e) => format!("Recall failed: {}", e),
        }
    }

    #[tool(description = "Capture a conversation turn into the memory system. Records user message and assistant response as L0 data, triggers background pipeline (L1 extraction -> L2 scene -> L3 persona).")]
    async fn tdai_capture(&self, Parameters(p): Parameters<CaptureParams>) -> String {
        let sk = self.resolve_session_key(p.session_key);
        let mut body = json!({
            "user_content": p.user_content,
            "assistant_content": p.assistant_content,
            "session_key": sk,
        });
        if let Some(sid) = p.session_id {
            body["session_id"] = json!(sid);
        }

        match self.post("/capture", body).await {
            Ok((true, data)) => format!(
                "Captured: {} message(s) recorded. Pipeline notified: {}",
                data["l0_recorded"].as_u64().unwrap_or(0),
                data["scheduler_notified"].as_bool().unwrap_or(false),
            ),
            Ok((false, data)) => format!("Capture error: {}", data),
            Err(e) => format!("Capture failed: {}", e),
        }
    }

    #[tool(description = "Signal end of a conversation session. Flushes buffered pipeline work (pending L1/L2 extraction). Call when conversation ends to persist all memories.")]
    async fn tdai_session_end(&self, Parameters(p): Parameters<SessionEndParams>) -> String {
        let sk = self.resolve_session_key(p.session_key);
        let body = json!({ "session_key": sk });

        match self.post("/session/end", body).await {
            Ok((true, data)) => format!(
                "Session \"{}\" ended. Pipeline flushed: {}",
                sk,
                data["flushed"].as_bool().unwrap_or(false),
            ),
            Ok((false, data)) => format!("Session end error: {}", data),
            Err(e) => format!("Session end failed: {}", e),
        }
    }

    #[tool(description = "Batch-import historical conversation data into the memory system. Processes through full L0->L1 pipeline. Use for migrating data or bootstrapping memory from logs.")]
    async fn tdai_seed(&self, Parameters(p): Parameters<SeedParams>) -> String {
        let parsed: Value = match serde_json::from_str(&p.data) {
            Ok(v) => v,
            Err(_) => return "Invalid JSON in data parameter".into(),
        };

        let mut body = json!({
            "data": parsed,
            "auto_fill_timestamps": p.auto_fill_timestamps.unwrap_or(true),
        });
        if let Some(sk) = p.session_key {
            body["session_key"] = json!(sk);
        }

        match self.post("/seed", body).await {
            Ok((true, data)) => format!(
                "Seed complete:\n  Sessions: {}\n  Rounds: {}\n  Messages: {}\n  L0 recorded: {}\n  Duration: {:.1}s\n  Output: {}",
                data["sessions_processed"].as_u64().unwrap_or(0),
                data["rounds_processed"].as_u64().unwrap_or(0),
                data["messages_processed"].as_u64().unwrap_or(0),
                data["l0_recorded"].as_u64().unwrap_or(0),
                data["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0,
                data["output_dir"].as_str().unwrap_or(""),
            ),
            Ok((false, data)) => format!("Seed error: {}", data),
            Err(e) => format!("Seed failed: {}", e),
        }
    }
}

#[tool_handler(name = "tencentdb-memory", version = "0.1.0")]
impl ServerHandler for TdaiServer {}

// ============================
// Entry point
// ============================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = TdaiServer::new();
    eprintln!(
        "[tencentdb-memory-mcp] Rust MCP server started (gateway: {})",
        server.config.gateway_url,
    );
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
