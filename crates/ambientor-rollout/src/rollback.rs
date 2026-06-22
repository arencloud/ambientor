use ambientor_types::{RolloutSpec, RolloutStage, RolloutStageType};
use kube::Client;

use crate::engine::RolloutError;
use crate::ingress::revert_ambient_ingress;
use crate::labels::unlabel_namespace_ambient;
use crate::policy::revert_translations_in_namespace;
use crate::restart::revert_rolling_restart_annotations;
use crate::waypoint::revert_waypoint;
use ambientor_mesh::unenroll_namespace_from_mesh;
use ambientor_types::MeshInstance;

/// Stages to revert when execution fails at `failed_at` (exclusive): indices `0..failed_at`, newest first.
pub fn stages_to_revert(
    spec: &RolloutSpec,
    failed_at: usize,
) -> impl Iterator<Item = &RolloutStage> {
    spec.stages.iter().take(failed_at).rev()
}

/// Undo Kubernetes changes from completed rollout stages (reverse order).
pub async fn revert_completed_stages(
    client: &Client,
    spec: &RolloutSpec,
    failed_at: usize,
    mesh: Option<&MeshInstance>,
) -> Result<Vec<String>, RolloutError> {
    let mut messages = Vec::new();
    for stage in stages_to_revert(spec, failed_at) {
        let msg = revert_stage(client, stage, mesh, spec).await?;
        messages.push(msg);
    }
    Ok(messages)
}

async fn revert_stage(
    client: &Client,
    stage: &RolloutStage,
    mesh: Option<&MeshInstance>,
    spec: &RolloutSpec,
) -> Result<String, RolloutError> {
    match stage.r#type {
        RolloutStageType::LabelNamespace => {
            for ns in &stage.namespaces {
                unlabel_namespace_ambient(client, ns).await?;
            }
            Ok(format!(
                "Removed ambient label from {} namespace(s)",
                stage.namespaces.len()
            ))
        }
        RolloutStageType::DeployWaypoint => {
            for ns in &stage.namespaces {
                revert_waypoint(client, ns).await?;
            }
            Ok(format!(
                "Reverted waypoint in {} namespace(s)",
                stage.namespaces.len()
            ))
        }
        RolloutStageType::TranslatePolicy => {
            let mut total = 0usize;
            for ns in &stage.namespaces {
                total += revert_translations_in_namespace(client, ns).await?;
            }
            Ok(format!("Removed {total} applied translation(s)"))
        }
        RolloutStageType::MigrateIngress => {
            let mut notes = Vec::new();
            for ns in &stage.namespaces {
                let msg = revert_ambient_ingress(
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
                total += revert_rolling_restart_annotations(client, ns).await?;
            }
            Ok(format!(
                "Cleared rolling-restart annotations on {total} Deployment(s)"
            ))
        }
        RolloutStageType::VerifyTraffic | RolloutStageType::DryRun => {
            Ok(format!("No resources to revert for {:?}", stage.r#type))
        }
        RolloutStageType::EnrollNamespace => {
            let Some(mesh) = mesh else {
                return Err(RolloutError::ExecutionFailed(
                    "rollback for EnrollNamespace requires resolved mesh target".into(),
                ));
            };
            let mut notes = Vec::new();
            for ns in &stage.namespaces {
                let steps = unenroll_namespace_from_mesh(client, ns, mesh)
                    .await
                    .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
                notes.extend(steps);
            }
            Ok(notes.join("; "))
        }
        RolloutStageType::RemoveInjection => {
            Ok("Injection labels restored via pre-migration snapshot on rollback".into())
        }
        RolloutStageType::InstallAmbientComponents => {
            Ok("InstallAmbientComponents is a no-op; nothing to revert".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_types::RolloutStage;

    fn stage(name: &str, t: RolloutStageType) -> RolloutStage {
        RolloutStage {
            name: name.into(),
            r#type: t,
            namespaces: vec!["ns".into()],
            requires_approval: false,
        }
    }

    #[test]
    fn revert_order_is_reverse_of_completed() {
        let spec = RolloutSpec {
            plan_ref: None,
            cluster_ref: None,
            auto_rollback: true,
            mesh_target: None,
            ambient_ingress_gateway: None,
            stages: vec![
                stage("dry", RolloutStageType::DryRun),
                stage("label", RolloutStageType::LabelNamespace),
                stage("wp", RolloutStageType::DeployWaypoint),
                stage("fail", RolloutStageType::VerifyTraffic),
            ],
        };
        let names: Vec<_> = stages_to_revert(&spec, 3)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, vec!["wp", "label", "dry"]);
    }
}
