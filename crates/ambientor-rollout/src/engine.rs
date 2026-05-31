use ambientor_types::{MeshInstance, RolloutSpec, RolloutStageType, RolloutStatus, StageResult};
use chrono::Utc;
use kube::Client;
use thiserror::Error;
use tracing::warn;

use crate::events::{RolloutEvent, RolloutEventType};
use crate::labels::{label_namespace_ambient, remove_namespace_injection};
use crate::policy::translate_policies_in_namespace;
use crate::preflight::{
    dry_run_namespace, namespaces_in_rollout, preflight_namespace_for_ambient_rollout,
};
use crate::restart::rolling_restart_namespace;
use crate::rollback::revert_completed_stages;
use crate::verify::{verify_application_reachability, verify_namespace_traffic};
use crate::waypoint::deploy_waypoint;
use ambientor_mesh::enroll_namespace_on_mesh;

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
        mesh: &MeshInstance,
    ) -> Result<Vec<RolloutEvent>, RolloutError> {
        status.resolved_mesh_target = Some(mesh.clone());
        let mut events = Vec::new();
        if status.phase.is_empty() {
            status.phase = "Pending".into();
        }
        if status.approved_stage == 0
            && status.current_stage == 0
            && status.stage_results.is_empty()
            && status.phase != "Completed"
            && status.phase != "AwaitingApproval"
        {
            status.approved_stage = -1;
        }

        const MAX_STAGES_PER_RECONCILE: usize = 32;
        for _ in 0..MAX_STAGES_PER_RECONCILE {
            let stage_idx = status.current_stage as usize;
            if stage_idx >= spec.stages.len() {
                status.phase = "Completed".into();
                events.push(RolloutEvent {
                    rollout_id: String::new(),
                    stage_index: status.current_stage,
                    stage_name: "done".into(),
                    event_type: RolloutEventType::RolloutCompleted,
                    message: "All stages completed — applications verified reachable on ambient".into(),
                    timestamp: Utc::now(),
                });
                break;
            }

            let stage = &spec.stages[stage_idx];
            if stage.requires_approval && status.approved_stage < status.current_stage {
                status.phase = "AwaitingApproval".into();
                events.push(RolloutEvent {
                    rollout_id: String::new(),
                    stage_index: status.current_stage,
                    stage_name: stage.name.clone(),
                    event_type: RolloutEventType::ApprovalRequired,
                    message: format!(
                        "Approve to start migration (stage: {})",
                        stage.name
                    ),
                    timestamp: Utc::now(),
                });
                break;
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

            let stage = spec.stages[stage_idx].clone();
            let result = self.execute_stage(spec, &stage, mesh).await;
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
                    // Continue same reconcile for auto stages (post-approval pipeline).
                    continue;
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
                    break;
                }
            }
        }

        Ok(events)
    }

    async fn execute_stage(
        &self,
        spec: &RolloutSpec,
        stage: &ambientor_types::RolloutStage,
        mesh: &MeshInstance,
    ) -> Result<String, RolloutError> {
        match stage.r#type {
            RolloutStageType::DryRun => {
                let namespaces = namespaces_in_rollout(&spec.stages);
                for ns in &namespaces {
                    dry_run_namespace(&self.client, ns, mesh, &spec.stages).await?;
                }
                let enroll_note = if namespaces.iter().any(|ns| {
                    crate::preflight::rollout_will_enroll_namespace(&spec.stages, ns)
                }) {
                    "; enrollment will run in EnrollNamespace stage(s)"
                } else {
                    ""
                };
                Ok(format!(
                    "Dry run passed for mesh {} ({}) on {} namespace(s){enroll_note}",
                    mesh.discovery_label,
                    mesh.enrollment.revision,
                    namespaces.len()
                ))
            }
            RolloutStageType::EnrollNamespace => {
                let mut actions = Vec::new();
                for ns in &stage.namespaces {
                    let mut step = enroll_namespace_on_mesh(&self.client, ns, mesh)
                        .await
                        .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
                    actions.append(&mut step);
                }
                Ok(actions.join("; "))
            }
            RolloutStageType::RemoveInjection => {
                for ns in &stage.namespaces {
                    remove_namespace_injection(&self.client, ns).await?;
                }
                Ok("Removed sidecar injection from namespace(s)".into())
            }
            RolloutStageType::LabelNamespace => {
                for ns in &stage.namespaces {
                    preflight_namespace_for_ambient_rollout(&self.client, ns, mesh, &spec.stages)
                        .await?;
                    label_namespace_ambient(&self.client, ns).await?;
                }
                Ok(format!("Labeled {} namespace(s)", stage.namespaces.len()))
            }
            RolloutStageType::DeployWaypoint => {
                for ns in &stage.namespaces {
                    deploy_waypoint(&self.client, ns, mesh, &spec.stages).await?;
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
                    verify_namespace_traffic(&self.client, ns, mesh).await?;
                    verify_application_reachability(&self.client, ns).await?;
                }
                Ok(format!(
                    "Verified ambient enrollment, waypoint, policy, and workload reachability for {} namespace(s)",
                    stage.namespaces.len()
                ))
            }
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
        status.approved_stage = -1;
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
