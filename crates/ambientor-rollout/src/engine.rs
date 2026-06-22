use ambientor_types::{MeshInstance, RolloutSpec, RolloutStageType, RolloutStatus, StageResult};
use chrono::Utc;
use kube::Client;
use thiserror::Error;
use tracing::warn;

use crate::events::{RolloutEvent, RolloutEventType};
use crate::labels::{
    label_namespace_ambient, remove_namespace_injection, restore_namespace_pre_migration,
    snapshot_namespace_pre_migration,
};
use crate::restart::rolling_restart_namespace;
use crate::ingress::{migrate_ambient_ingress, revert_ambient_ingress};
use crate::policy::translate_policies_in_namespace;
use crate::preflight::{
    dry_run_namespace, namespaces_in_rollout, preflight_namespace_for_ambient_rollout,
};
use crate::rollback::revert_completed_stages;
use crate::verify::{verify_application_reachability, verify_namespace_traffic};
use crate::waypoint::deploy_waypoint;
use ambientor_mesh::{ensure_istiod_trusts_ztunnel, enroll_namespace_on_mesh};

pub const FIELD_MANAGER: &str = "ambientor.io";

/// True when a one-time human approval has authorized the full pipeline.
pub fn pipeline_approved(status: &RolloutStatus, stage_count: usize) -> bool {
    if stage_count == 0 {
        return true;
    }
    let last = stage_count.saturating_sub(1) as i32;
    status.approved_stage >= last
}

/// Whether the UI should offer an approval action (initial gate only).
pub fn rollout_awaiting_approval(status: &RolloutStatus, stage_count: usize) -> bool {
    match status.phase.as_str() {
        "Completed" | "Failed" | "RolledBack" => return false,
        _ => {}
    }
    if pipeline_approved(status, stage_count) {
        return false;
    }
    status.phase == "AwaitingApproval"
        || (status.current_stage == 0 && status.approved_stage < 0 && status.stage_results.is_empty())
}

pub struct RolloutEngine;

#[derive(Debug, Error)]
pub enum RolloutError {
    #[error("stage {0} requires approval but approved_stage is {1}")]
    ApprovalRequired(i32, i32),
    #[error("stage execution failed: {0}")]
    ExecutionFailed(String),
    #[error("kubernetes error: {0}")]
    Kube(#[from] kube::Error),
}

impl RolloutEngine {
    pub fn new() -> Self {
        Self
    }

    pub async fn reconcile(
        &self,
        client: &Client,
        spec: &RolloutSpec,
        status: &mut RolloutStatus,
        mesh: &MeshInstance,
    ) -> Result<Vec<RolloutEvent>, RolloutError> {
        status.resolved_mesh_target = Some(mesh.clone());
        let mut events = Vec::new();
        if status.phase.is_empty() {
            status.phase = "Pending".into();
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
            if stage.requires_approval && !pipeline_approved(status, spec.stages.len()) {
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
            let result = self.execute_stage(client, spec, &stage, mesh).await;
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
                        self.rollback(
                            client,
                            spec,
                            status,
                            status.current_stage as usize,
                            mesh,
                            &mut events,
                        )
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
        client: &Client,
        spec: &RolloutSpec,
        stage: &ambientor_types::RolloutStage,
        mesh: &MeshInstance,
    ) -> Result<String, RolloutError> {
        match stage.r#type {
            RolloutStageType::DryRun => {
                let namespaces = namespaces_in_rollout(&spec.stages);
                for ns in &namespaces {
                    dry_run_namespace(client, ns, mesh, &spec.stages).await?;
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
                    snapshot_namespace_pre_migration(client, ns).await?;
                    let mut step = enroll_namespace_on_mesh(client, ns, mesh)
                        .await
                        .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
                    actions.append(&mut step);
                }
                Ok(actions.join("; "))
            }
            RolloutStageType::RemoveInjection => {
                for ns in &stage.namespaces {
                    remove_namespace_injection(client, ns).await?;
                }
                Ok("Removed sidecar injection from namespace(s)".into())
            }
            RolloutStageType::LabelNamespace => {
                for ns in &stage.namespaces {
                    preflight_namespace_for_ambient_rollout(client, ns, mesh, &spec.stages)
                        .await?;
                    label_namespace_ambient(client, ns).await?;
                }
                Ok(format!("Labeled {} namespace(s)", stage.namespaces.len()))
            }
            RolloutStageType::DeployWaypoint => {
                for ns in &stage.namespaces {
                    deploy_waypoint(client, ns, mesh, &spec.stages).await?;
                }
                Ok(format!(
                    "Deployed waypoint Gateway for {} namespace(s)",
                    stage.namespaces.len()
                ))
            }
            RolloutStageType::TranslatePolicy => {
                let mut total = 0usize;
                for ns in &stage.namespaces {
                    total += translate_policies_in_namespace(client, ns).await?;
                }
                Ok(format!("Applied {total} HTTPRoute translation(s)"))
            }
            RolloutStageType::MigrateIngress => {
                let mut notes = Vec::new();
                for ns in &stage.namespaces {
                    let msg = migrate_ambient_ingress(
                        client,
                        ns,
                        mesh,
                        spec.ambient_ingress_gateway.as_ref(),
                    )
                    .await?;
                    notes.push(msg);
                }
                Ok(notes.join("; "))
            }
            RolloutStageType::RollingRestart => {
                let mut total = 0usize;
                for ns in &stage.namespaces {
                    total += rolling_restart_namespace(client, ns).await?;
                }
                Ok(format!(
                    "Triggered rolling restart on {total} Deployment(s)"
                ))
            }
            RolloutStageType::VerifyTraffic => {
                for ns in &stage.namespaces {
                    verify_namespace_traffic(client, ns, mesh).await?;
                    verify_application_reachability(client, ns).await?;
                }
                Ok(format!(
                    "Verified ambient enrollment, waypoint, policy, and workload reachability for {} namespace(s)",
                    stage.namespaces.len()
                ))
            }
            RolloutStageType::InstallAmbientComponents => {
                let msg = ensure_istiod_trusts_ztunnel(client, mesh)
                    .await
                    .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
                Ok(format!("Ambient components check passed ({msg})"))
            }
        }
    }

    async fn rollback(
        &self,
        client: &Client,
        spec: &RolloutSpec,
        status: &mut RolloutStatus,
        failed_at: usize,
        mesh: &MeshInstance,
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

        let revert_messages =
            revert_completed_stages(client, spec, failed_at, Some(mesh)).await?;
        let finalize = self.finalize_rollback_namespaces(client, spec, status, mesh).await?;
        let mut summary = revert_messages;
        if !finalize.is_empty() {
            summary.push(finalize);
        }
        let summary = summary.join("; ");
        status.current_stage = failed_at as i32;
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

    /// After stage reverts, restore pre-migration label snapshots and restart workloads so
    /// sidecar injection matches the reverted namespace labels.
    async fn finalize_rollback_namespaces(
        &self,
        client: &Client,
        spec: &RolloutSpec,
        status: &RolloutStatus,
        mesh: &MeshInstance,
    ) -> Result<String, RolloutError> {
        let namespaces = namespaces_in_rollout(&spec.stages);
        let mesh_ref = status
            .resolved_mesh_target
            .as_ref()
            .unwrap_or(mesh);
        let mut notes = Vec::new();
        for ns in namespaces {
            if restore_namespace_pre_migration(client, &ns).await? {
                notes.push(format!("restored labels on {ns}"));
            }
            match revert_ambient_ingress(
                client,
                &ns,
                Some(mesh_ref),
                spec.ambient_ingress_gateway.as_ref(),
            )
            .await
            {
                Ok(msg) if msg != "no ingress migration resources to revert" => notes.push(msg),
                Ok(_) => {}
                Err(e) => warn!(namespace = %ns, error = %e, "ingress cleanup during rollback skipped"),
            }
            match rolling_restart_namespace(client, &ns).await {
                Ok(n) if n > 0 => notes.push(format!("restarted {n} Deployment(s) in {ns}")),
                Ok(_) => {}
                Err(e) => warn!(namespace = %ns, error = %e, "post-rollback restart skipped"),
            }
        }
        Ok(notes.join("; "))
    }
}

impl Default for RolloutEngine {
    fn default() -> Self {
        Self::new()
    }
}
