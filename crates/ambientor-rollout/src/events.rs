use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RolloutEvent {
    pub rollout_id: String,
    pub stage_index: i32,
    pub stage_name: String,
    pub event_type: RolloutEventType,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutEventType {
    StageStarted,
    StageCompleted,
    StageFailed,
    ApprovalRequired,
    RollbackStarted,
    RollbackCompleted,
    RolloutCompleted,
}
