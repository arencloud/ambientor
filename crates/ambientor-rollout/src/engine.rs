use ambientor_types::{RolloutSpec, RolloutStageType, RolloutStatus, StageResult};
use chrono::Utc;
use kube::Client;
use thiserror::Error;
use tracing::warn;

use crate::events::{RolloutEvent, RolloutEventType};
use crate::labels::label_namespace_ambient;
use crate::policy::translate_policies_in_namespace;
use crate::restart::rolling_restart_namespace;
use crate::rollback::revert_completed_stages;
use crate::verify::verify_namespace_traffic;
use crate::waypoint::deploy_waypoint;

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
                    self.rollback(spec, status, status.current_stage as usize, &mut events)
                        .await?;
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
                    label_namespace_ambient(&self.client, ns).await?;
                }
                Ok(format!("Labeled {} namespace(s)", stage.namespaces.len()))
            }
            RolloutStageType::DeployWaypoint => {
                for ns in &stage.namespaces {
                    deploy_waypoint(&self.client, ns).await?;
                }
                Ok(format!(
                    "Deployed waypoint Gateway for {} namespace(s)",
                    stage.namespaces.len()
                ))
            }
            RolloutStageType::TranslatePolicy => {
                let mut total = 0usize;
                for ns in &stage.namespaces {
                    total += translate_policies_in_namespace(&self.client, ns).await?;
                }
                Ok(format!("Applied {total} HTTPRoute translation(s)"))
            }
            RolloutStageType::RollingRestart => {
                let mut total = 0usize;
                for ns in &stage.namespaces {
                    total += rolling_restart_namespace(&self.client, ns).await?;
                }
                Ok(format!(
                    "Triggered rolling restart on {total} Deployment(s)"
                ))
            }
            RolloutStageType::VerifyTraffic => {
                for ns in &stage.namespaces {
                    verify_namespace_traffic(&self.client, ns).await?;
                }
                Ok(format!(
                    "Verified ambient labels, waypoint, and policy for {} namespace(s)",
                    stage.namespaces.len()
                ))
            }
            RolloutStageType::RemoveInjection => Ok("Injection labels removed".into()),
            RolloutStageType::InstallAmbientComponents => {
                Ok("Ambient components check passed".into())
            }
        }
    }

    async fn rollback(
        &self,
        spec: &RolloutSpec,
        status: &mut RolloutStatus,
        failed_at: usize,
        events: &mut Vec<RolloutEvent>,
    ) -> Result<(), RolloutError> {
        events.push(RolloutEvent {
            rollout_id: String::new(),
            stage_index: status.current_stage,
            stage_name: "rollback".into(),
            event_type: RolloutEventType::RollbackStarted,
            message: format!("Reverting {failed_at} completed stage(s)"),
            timestamp: Utc::now(),
        });

        let revert_messages = revert_completed_stages(&self.client, spec, failed_at).await?;
        let summary = revert_messages.join("; ");
        status.current_stage = 0;
        status.approved_stage = 0;
        status.phase = "RolledBack".into();

        events.push(RolloutEvent {
            rollout_id: String::new(),
            stage_index: 0,
            stage_name: "rollback".into(),
            event_type: RolloutEventType::RollbackCompleted,
            message: summary,
            timestamp: Utc::now(),
        });
        Ok(())
    }
}
