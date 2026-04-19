use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Roles in a conversation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "assistant" => Role::Assistant,
            _ => Role::User,
        }
    }
}

/// A single chat message
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: Role,
    pub content: String,
    pub created_at: DateTime<Utc>,
    /// Monotonically increasing version counter; bumped on every token append.
    /// The markdown renderer uses this to know when to re-parse.
    pub version: u64,
}

impl ChatMessage {
    pub fn new(conversation_id: &str, role: Role, content: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.to_string(),
            role,
            content: content.to_string(),
            created_at: Utc::now(),
            version: 0,
        }
    }

    /// Append a streaming token and bump the version so the UI knows to re-render.
    pub fn append_token(&mut self, token: &str) {
        self.content.push_str(token);
        self.version = self.version.wrapping_add(1);
    }
}

/// Metadata for a conversation shown in the sidebar
#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub system_prompt: String,
    pub model_id: String,
    pub region: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    pub fn new(title: &str, model_id: &str, region: &str) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.to_string(),
            system_prompt: String::new(),
            model_id: model_id.to_string(),
            region: region.to_string(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Available Bedrock models
pub const MODELS: &[(&str, &str)] = &[
    ("Claude Sonnet 4", "us.anthropic.claude-sonnet-4-20250514-v1:0"),
    ("Claude Sonnet 3.5 v2", "us.anthropic.claude-3-5-sonnet-20241022-v2:0"),
    ("Claude Haiku 3.5", "us.anthropic.claude-3-5-haiku-20241022-v1:0"),
    ("Llama 3.1 70B", "us.meta.llama3-1-70b-instruct-v1:0"),
    ("Llama 3.1 8B", "us.meta.llama3-1-8b-instruct-v1:0"),
];

/// Available AWS regions for Bedrock
pub const REGIONS: &[&str] = &[
    "us-east-1",
    "us-west-2",
    "eu-west-1",
    "ap-northeast-1",
];
