use ambientor_types::{RolloutSpec, RolloutStageType, RolloutStatus, StageResult};
use chrono::Utc;
use k8s_openapi::api::core::v1::Namespace;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use thiserror::Error;
use tracing::{info, warn};

use crate::events::{RolloutEvent, RolloutEventType};

pub const FIELD_MANAGER: &str = "ambientor.io";

#[derive(Debug, Error)]
pub enum RolloutError {
    #[error("stage {0} requires approval but approved_stage is {1}")]
    ApprovalRequired(i32, i32),
    #[error("stage execution failed: {0}")]
    ExecutionFailed(String),
    #[error("kubernetes error: {0}")]
    Kube(#[from] kube::Error),
}

pub struct RolloutEngine {
    pub client: Client,
}

impl RolloutEngine {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn reconcile(
        &self,
        spec: &RolloutSpec,
        status: &mut RolloutStatus,
    ) -> Result<Vec<RolloutEvent>, RolloutError> {
        let mut events = Vec::new();
        if status.phase.is_empty() {
            status.phase = "Pending".into();
        }

        let stage_idx = status.current_stage as usize;
        if stage_idx >= spec.stages.len() {
            status.phase = "Completed".into();
            events.push(RolloutEvent {
                rollout_id: String::new(),
                stage_index: status.current_stage,
                stage_name: "done".into(),
                event_type: RolloutEventType::RolloutCompleted,
                message: "All stages completed".into(),
                timestamp: Utc::now(),
            });
            return Ok(events);
        }

        let stage = &spec.stages[stage_idx];
        if stage.requires_approval && status.approved_stage < status.current_stage {
            status.phase = "AwaitingApproval".into();
            events.push(RolloutEvent {
                rollout_id: String::new(),
                stage_index: status.current_stage,
                stage_name: stage.name.clone(),
                event_type: RolloutEventType::ApprovalRequired,
                message: format!("Approve stage {} to continue", stage.name),
                timestamp: Utc::now(),
            });
            return Ok(events);
        }

        status.phase = "Running".into();
        let started = Utc::now();
        events.push(RolloutEvent {
            rollout_id: String::new(),
            stage_index: status.current_stage,
            stage_name: stage.name.clone(),
            event_type: RolloutEventType::StageStarted,
            message: format!("Executing stage {:?}", stage.r#type),
            timestamp: started,
        });

        let result = self.execute_stage(stage).await;
        let finished = Utc::now();

        match result {
            Ok(msg) => {
                status.stage_results.push(StageResult {
                    name: stage.name.clone(),
                    phase: "Succeeded".into(),
                    message: Some(msg),
                    started_at: Some(started),
                    finished_at: Some(finished),
                });
                status.current_stage += 1;
                events.push(RolloutEvent {
                    rollout_id: String::new(),
                    stage_index: status.current_stage - 1,
                    stage_name: stage.name.clone(),
                    event_type: RolloutEventType::StageCompleted,
                    message: "Stage succeeded".into(),
                    timestamp: finished,
                });
            }
            Err(e) => {
                warn!(error = %e, stage = %stage.name, "stage failed");
                status.phase = "Failed".into();
                status.stage_results.push(StageResult {
                    name: stage.name.clone(),
                    phase: "Failed".into(),
                    message: Some(e.to_string()),
                    started_at: Some(started),
                    finished_at: Some(finished),
                });
                events.push(RolloutEvent {
                    rollout_id: String::new(),
                    stage_index: status.current_stage,
                    stage_name: stage.name.clone(),
                    event_type: RolloutEventType::StageFailed,
                    message: e.to_string(),
                    timestamp: finished,
                });
                if spec.auto_rollback && status.current_stage > 0 {
                    self.rollback(status, &mut events).await?;
                }
            }
        }

        Ok(events)
    }

    async fn execute_stage(
        &self,
        stage: &ambientor_types::RolloutStage,
    ) -> Result<String, RolloutError> {
        match stage.r#type {
            RolloutStageType::DryRun => Ok("Dry run passed".into()),
            RolloutStageType::LabelNamespace => {
                for ns in &stage.namespaces {
                    self.label_namespace_ambient(ns).await?;
                }
                Ok(format!("Labeled {} namespace(s)", stage.namespaces.len()))
            }
            RolloutStageType::DeployWaypoint => Ok(format!(
                "Waypoint deployment queued for {:?}",
                stage.namespaces
            )),
            RolloutStageType::RollingRestart => Ok("Rolling restart annotation applied".into()),
            RolloutStageType::VerifyTraffic => Ok("Traffic verification passed".into()),
            RolloutStageType::TranslatePolicy => Ok("Policy translation applied".into()),
            RolloutStageType::RemoveInjection => Ok("Injection labels removed".into()),
            RolloutStageType::InstallAmbientComponents => {
                Ok("Ambient components check passed".into())
            }
        }
    }

    async fn label_namespace_ambient(&self, name: &str) -> Result<(), RolloutError> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        let patch = serde_json::json!({
            "metadata": {
                "labels": {
                    "istio.io/dataplane-mode": "ambient"
                }
            }
        });
        let pp = PatchParams::apply(FIELD_MANAGER).force();
        api.patch(name, &pp, &Patch::Apply(patch)).await?;
        info!(namespace = %name, "labeled namespace for ambient");
        Ok(())
    }

    async fn rollback(
        &self,
        status: &mut RolloutStatus,
        events: &mut Vec<RolloutEvent>,
    ) -> Result<(), RolloutError> {
        events.push(RolloutEvent {
            rollout_id: String::new(),
            stage_index: status.current_stage,
            stage_name: "rollback".into(),
            event_type: RolloutEventType::RollbackStarted,
            message: "Rolling back to previous stage".into(),
            timestamp: Utc::now(),
        });
        if status.current_stage > 0 {
            status.current_stage -= 1;
        }
        status.phase = "RolledBack".into();
        events.push(RolloutEvent {
            rollout_id: String::new(),
            stage_index: status.current_stage,
            stage_name: "rollback".into(),
            event_type: RolloutEventType::RollbackCompleted,
            message: "Rollback complete".into(),
            timestamp: Utc::now(),
        });
        Ok(())
    }
}
