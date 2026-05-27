use ambientor_types::{RolloutSpec, RolloutStage, RolloutStageType};
use kube::Client;
use tracing::warn;

use crate::engine::RolloutError;
use crate::labels::unlabel_namespace_ambient;
use crate::policy::revert_translations_in_namespace;
use crate::waypoint::revert_waypoint;

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
) -> Result<Vec<String>, RolloutError> {
    let mut messages = Vec::new();
    for stage in stages_to_revert(spec, failed_at) {
        let msg = revert_stage(client, stage).await?;
        messages.push(msg);
    }
    Ok(messages)
}

async fn revert_stage(client: &Client, stage: &RolloutStage) -> Result<String, RolloutError> {
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
        RolloutStageType::RollingRestart => {
            warn!(
                stage = %stage.name,
                "rolling restart cannot be reverted; workloads keep restarted pods"
            );
            Ok("Rolling restart not reverted (annotation-only)".into())
        }
        RolloutStageType::VerifyTraffic | RolloutStageType::DryRun => {
            Ok(format!("No resources to revert for {:?}", stage.r#type))
        }
        RolloutStageType::RemoveInjection | RolloutStageType::InstallAmbientComponents => {
            Ok(format!("No rollback implemented for {:?}", stage.r#type))
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
            auto_rollback: true,
            mesh_target: None,
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
