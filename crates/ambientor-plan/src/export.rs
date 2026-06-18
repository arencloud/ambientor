//! Multi-document YAML export for GitOps review.

use ambientor_types::{
    MigrationPlan, MigrationPlanSpec, MigrationPlanStatus, PolicyTranslation, RolloutSpec,
};
use serde_json::json;

/// Build an in-memory `MigrationPlan` CR for export (local CLI or tests).
pub fn migration_plan_cr(name: &str, namespace: &str, spec: MigrationPlanSpec) -> MigrationPlan {
    let wave_count = spec.waves.len() as i32;
    MigrationPlan {
        metadata: kube::api::ObjectMeta {
            name: Some(name.into()),
            namespace: Some(namespace.into()),
            ..Default::default()
        },
        spec,
        status: Some(MigrationPlanStatus {
            phase: "Ready".into(),
            approved: false,
            wave_count,
            selected_count: None,
            cluster_ref: None,
        }),
    }
}

/// Build a multi-document YAML bundle: MigrationPlan, policy translations, rollout preview.
pub fn build_export_yaml(
    plan: &MigrationPlan,
    translations: &[PolicyTranslation],
    rollout: &RolloutSpec,
) -> Result<String, String> {
    let mut parts = Vec::new();

    let name = plan
        .metadata
        .name
        .as_deref()
        .ok_or_else(|| "plan missing metadata.name".to_string())?;
    let namespace = plan.metadata.namespace.as_deref().unwrap_or("default");

    let plan_doc = json!({
        "apiVersion": "ambientor.io/v1alpha1",
        "kind": "MigrationPlan",
        "metadata": {
            "name": name,
            "namespace": namespace,
        },
        "spec": plan.spec,
    });
    let mut plan_yaml = serde_yaml::to_string(&plan_doc).map_err(|e| e.to_string())?;
    if let Some(status) = plan.status.as_ref() {
        let status_doc = json!({
            "phase": status.phase,
            "approved": status.approved,
            "waveCount": status.wave_count,
            "selectedCount": status.selected_count,
            "clusterRef": status.cluster_ref,
        });
        plan_yaml.push_str(&format!(
            "\n# Live status (sync portal / CLI / GitOps via status subresource)\nstatus:\n{}",
            serde_yaml::to_string(&status_doc).map_err(|e| e.to_string())?
        ));
    }
    parts.push(plan_yaml);

    for pt in translations {
        let pt_name = pt.metadata.name.as_deref().unwrap_or("translation");
        if let Some(manifest) = pt
            .status
            .as_ref()
            .and_then(|s| s.suggested_manifest.as_ref())
        {
            parts.push(format!(
                "# PolicyTranslation: {}/{} (phase: {})\n{manifest}",
                namespace,
                pt_name,
                pt.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("?")
            ));
        }
    }

    let rollout_doc = json!({
        "apiVersion": "ambientor.io/v1alpha1",
        "kind": "Rollout",
        "metadata": {
            "name": format!("{name}-rollout"),
            "namespace": namespace,
            "annotations": {
                "ambientor.io/preview": "generated from MigrationPlan; not applied automatically"
            }
        },
        "spec": rollout,
    });
    parts.push(format!(
        "# Rollout preview — one human approval on stage 0 runs the full pipeline.\n# GitOps: patch status.approvedStage to currentStage when phase is AwaitingApproval.\n{}",
        serde_yaml::to_string(&rollout_doc).map_err(|e| e.to_string())?
    ));

    Ok(parts.join("\n---\n"))
}

#[cfg(test)]
mod tests {
    use ambientor_types::{MigrationPlanSpec, MigrationWave};

    use super::*;
    use crate::plan_to_rollout;

    #[test]
    fn export_contains_plan_and_preview() {
        let plan = MigrationPlan {
            spec: MigrationPlanSpec {
                assessment_ref: Some("lab-assessment".into()),
                selected_namespaces: vec![],
                cluster_ref: None,
                display_name: None,
                target_mesh_mode: "ambient".into(),
                mesh_target: None,
                ambient_ingress_gateway: None,
                waves: vec![MigrationWave {
                    name: "wave-1".into(),
                    namespaces: vec!["bookinfo".into()],
                    prerequisites: vec![],
                    policy_tasks: vec![],
                }],
            },
            status: None,
            metadata: kube::api::ObjectMeta {
                name: Some("lab-assessment-plan".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
        };
        let rollout = plan_to_rollout(&plan.spec);
        let yaml = build_export_yaml(&plan, &[], &rollout).unwrap();
        assert!(yaml.contains("kind: MigrationPlan"));
        assert!(yaml.contains("kind: Rollout"));
        assert!(yaml.contains("wave-1"));
    }
}
