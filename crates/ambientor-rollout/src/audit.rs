use ambientor_types::AuditEvent;
use chrono::Utc;
use uuid::Uuid;

use crate::events::{RolloutEvent, RolloutEventType};

/// Canonical audit resource key for a namespaced Rollout CR.
pub fn rollout_resource(namespace: &str, name: &str) -> String {
    format!("rollout/{namespace}/{name}")
}

/// Build an audit row from an operator-emitted rollout event.
pub fn audit_from_rollout_event(
    namespace: &str,
    name: &str,
    actor: &str,
    event: &RolloutEvent,
) -> AuditEvent {
    let (action, outcome) = match event.event_type {
        RolloutEventType::StageStarted => ("rollout.stage.apply", "started"),
        RolloutEventType::StageCompleted => ("rollout.stage.apply", "succeeded"),
        RolloutEventType::StageFailed => ("rollout.stage.apply", "failed"),
        RolloutEventType::ApprovalRequired => ("rollout.approval.required", "pending"),
        RolloutEventType::RollbackStarted => ("rollout.rollback", "started"),
        RolloutEventType::RollbackCompleted => ("rollout.rollback", "completed"),
        RolloutEventType::RolloutCompleted => ("rollout.complete", "succeeded"),
    };
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: event.timestamp,
        actor: actor.to_string(),
        action: action.into(),
        resource: rollout_resource(namespace, name),
        outcome: outcome.into(),
        details: Some(serde_json::json!({
            "stageIndex": event.stage_index,
            "stageName": event.stage_name,
            "message": event.message,
            "eventType": format!("{:?}", event.event_type),
        })),
    }
}

/// Audit row for a human/API approval of a rollout stage.
pub fn audit_rollout_approve(
    namespace: &str,
    name: &str,
    actor: &str,
    stage_index: i32,
) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        actor: actor.to_string(),
        action: "rollout.approve".into(),
        resource: rollout_resource(namespace, name),
        outcome: "succeeded".into(),
        details: Some(serde_json::json!({ "stageIndex": stage_index })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn maps_stage_completed() {
        let event = RolloutEvent {
            rollout_id: String::new(),
            stage_index: 1,
            stage_name: "wave-label".into(),
            event_type: RolloutEventType::StageCompleted,
            message: "ok".into(),
            timestamp: Utc::now(),
        };
        let audit = audit_from_rollout_event("bookinfo", "plan-rollout", "operator", &event);
        assert_eq!(audit.action, "rollout.stage.apply");
        assert_eq!(audit.outcome, "succeeded");
        assert_eq!(audit.resource, "rollout/bookinfo/plan-rollout");
    }
}
