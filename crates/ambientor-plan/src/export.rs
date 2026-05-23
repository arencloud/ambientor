//! Multi-document YAML export for GitOps review.

use ambientor_types::{MigrationPlan, PolicyTranslation, RolloutSpec};
use serde_json::json;

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
    parts.push(serde_yaml::to_string(&plan_doc).map_err(|e| e.to_string())?);

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
        "# Rollout preview (approval-gated in Phase 3)\n{}",
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
                target_mesh_mode: "ambient".into(),
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
