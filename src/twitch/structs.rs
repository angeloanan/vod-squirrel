use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub length_seconds: u64,
    pub view_count: u64,
    pub status: Status,
    pub game: Game,
    pub owner: Channel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Status {
    RECORDED,
    RECORDING,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub login: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub display_name: String,
}
