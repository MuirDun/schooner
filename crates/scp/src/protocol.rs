use serde::{Deserialize, Serialize};

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Cmd { request: String },
}

#[derive(Serialize, Deserialize)]
pub struct CommandResponse {
    pub transaction_id: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub state: SessionState,
    pub body: serde_json::Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct SessionState {
    pub variables: Vec<String>,
}


