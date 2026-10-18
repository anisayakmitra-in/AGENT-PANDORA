use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerMessageStatus {
    Queued,
    Delivered,
    Read,
    Responded,
}

impl PeerMessageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Delivered => "delivered",
            Self::Read => "read",
            Self::Responded => "responded",
        }
    }
}

impl std::str::FromStr for PeerMessageStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "delivered" => Ok(Self::Delivered),
            "read" => Ok(Self::Read),
            "responded" => Ok(Self::Responded),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentPersona {
    pub handle: String,
    pub display_name: String,
    pub role: String,
    pub system_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub avatar_seed: String,
    pub capabilities: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl AgentPersona {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: impl Into<String>,
        display_name: impl Into<String>,
        role: impl Into<String>,
        system_prompt: impl Into<String>,
        model: Option<String>,
        avatar_seed: impl Into<String>,
        capabilities: Vec<String>,
        now: u64,
    ) -> Self {
        let mut clean_handle = handle.into().trim().to_lowercase();
        if clean_handle.starts_with('@') {
            clean_handle.remove(0);
        }
        Self {
            handle: clean_handle,
            display_name: display_name.into(),
            role: role.into(),
            system_prompt: system_prompt.into(),
            model,
            avatar_seed: avatar_seed.into(),
            capabilities,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PeerMessage {
    pub message_id: String,
    pub thread_id: String,
    pub from_handle: String,
    pub to_handle: String,
    pub subject: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_json: Option<serde_json::Value>,
    pub status: PeerMessageStatus,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PeerThread {
    pub thread_id: String,
    pub subject: String,
    pub participants: Vec<String>,
    pub message_count: usize,
    pub last_activity: u64,
    pub messages: Vec<PeerMessage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PeerDispatchReceipt {
    pub message_id: String,
    pub thread_id: String,
    pub from_handle: String,
    pub to_handle: String,
    pub status: PeerMessageStatus,
    pub dispatched_at: u64,
    pub delivered: bool,
}
