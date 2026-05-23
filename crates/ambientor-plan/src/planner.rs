use std::collections::BTreeSet;

use ambientor_core::inventory::AssessmentResult;
use ambientor_types::{
    AmbientAssessmentStatus, AssessmentScores, Finding, MigrationPlanSpec, MigrationWave,
    PolicyTask, RolloutSpec, RolloutStage, RolloutStageType,
};

/// Stable MigrationPlan name for an AmbientAssessment.
pub fn plan_name_for_assessment(assessment_name: &str) -> String {
    format!("{assessment_name}-plan")
}

/// Stable PolicyTranslation CR name for a VirtualService.
pub fn translation_name_for_vs(vs_name: &str) -> String {
    format!("{vs_name}-translation")
}

/// Namespace list for wave planning (defaults to `default` when no finding namespaces).
pub fn namespaces_from_findings(findings: &[Finding]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for f in findings {
        if let Some(ns) = &f.namespace {
            set.insert(ns.clone());
        }
    }
    if set.is_empty() {
        return vec!["default".into()];
    }
    set.into_iter().collect()
}

/// Build an assessment result from a completed AmbientAssessment status.
pub fn assessment_result_from_status(status: &AmbientAssessmentStatus) -> AssessmentResult {
    AssessmentResult {
        findings: status.findings.clone(),
        scores: AssessmentScores {
            readiness: status.readiness_score,
            sidecar_dependency: status.sidecar_dependency_score,
            traffic_compatibility: status.traffic_compatibility_score,
            overall: status.overall_score,
        },
        summary: status.summary.clone().unwrap_or_default(),
    }
}

/// Build a migration plan from assessment results.
pub fn build_plan(assessment: &AssessmentResult, namespaces: &[String]) -> MigrationPlanSpec {
    let waves = plan_waves(namespaces, assessment);
    MigrationPlanSpec {
        assessment_ref: None,
        target_mesh_mode: "ambient".into(),
        waves,
    }
}

pub fn plan_waves(namespaces: &[String], assessment: &AssessmentResult) -> Vec<MigrationWave> {
    let mut ordered: Vec<_> = namespaces.to_vec();
    ordered.sort();

    let blocker_ns: std::collections::HashSet<_> = assessment
        .findings
        .iter()
        .filter(|f| matches!(f.severity, ambientor_types::FindingSeverity::Blocker))
        .filter_map(|f| f.namespace.clone())
        .collect();

    let ready: Vec<_> = ordered
        .iter()
        .filter(|ns| !blocker_ns.contains(*ns))
        .cloned()
        .collect();
    let blocked: Vec<_> = ordered
        .iter()
        .filter(|ns| blocker_ns.contains(*ns))
        .cloned()
        .collect();

    let mut waves = Vec::new();
    if !ready.is_empty() {
        waves.push(MigrationWave {
            name: "wave-1-canary".into(),
            namespaces: ready.first().map(|n| vec![n.clone()]).unwrap_or_default(),
            prerequisites: vec![
                "ztunnel DaemonSet healthy".into(),
                "istio-cni ambient mode enabled".into(),
            ],
            policy_tasks: vec![],
        });
        if ready.len() > 1 {
            waves.push(MigrationWave {
                name: "wave-2-expand".into(),
                namespaces: ready[1..].to_vec(),
                prerequisites: vec!["wave-1 verification passed".into()],
                policy_tasks: policy_tasks_from_findings(assessment),
            });
        }
    }
    if !blocked.is_empty() {
        waves.push(MigrationWave {
            name: "wave-blocked".into(),
            namespaces: blocked,
            prerequisites: vec!["Resolve blocker findings before rollout".into()],
            policy_tasks: vec![],
        });
    }
    waves
}

fn policy_tasks_from_findings(assessment: &AssessmentResult) -> Vec<PolicyTask> {
    assessment
        .findings
        .iter()
        .filter(|f| f.id.starts_with("traffic."))
        .map(|f| PolicyTask {
            kind: "review".into(),
            name: f.id.clone(),
            namespace: f.namespace.clone().unwrap_or_default(),
            action: f
                .remediation
                .clone()
                .unwrap_or_else(|| "Review policy translation".into()),
        })
        .collect()
}

pub fn plan_to_rollout(plan: &MigrationPlanSpec) -> RolloutSpec {
    let mut stages = vec![RolloutStage {
        name: "preflight-dry-run".into(),
        r#type: RolloutStageType::DryRun,
        namespaces: vec![],
        requires_approval: false,
    }];

    for (i, wave) in plan.waves.iter().enumerate() {
        if wave.name == "wave-blocked" {
            continue;
        }
        stages.push(RolloutStage {
            name: format!("{}-label", wave.name),
            r#type: RolloutStageType::LabelNamespace,
            namespaces: wave.namespaces.clone(),
            requires_approval: true,
        });
        stages.push(RolloutStage {
            name: format!("{}-waypoint", wave.name),
            r#type: RolloutStageType::DeployWaypoint,
            namespaces: wave.namespaces.clone(),
            requires_approval: i > 0,
        });
        stages.push(RolloutStage {
            name: format!("{}-translate", wave.name),
            r#type: RolloutStageType::TranslatePolicy,
            namespaces: wave.namespaces.clone(),
            requires_approval: true,
        });
        stages.push(RolloutStage {
            name: format!("{}-restart", wave.name),
            r#type: RolloutStageType::RollingRestart,
            namespaces: wave.namespaces.clone(),
            requires_approval: true,
        });
        stages.push(RolloutStage {
            name: format!("{}-verify", wave.name),
            r#type: RolloutStageType::VerifyTraffic,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
        });
    }

    RolloutSpec {
        plan_ref: None,
        auto_rollback: true,
        stages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_core::inventory::AssessmentResult;
    use ambientor_types::{Finding, FindingCategory, FindingSeverity};

    #[test]
    fn namespaces_default_when_empty() {
        assert_eq!(namespaces_from_findings(&[]), vec!["default"]);
    }

    #[test]
    fn rollout_includes_translate_after_waypoint() {
        let plan = build_plan(
            &AssessmentResult {
                findings: vec![],
                scores: Default::default(),
                summary: Default::default(),
            },
            &["bookinfo".into()],
        );
        let rollout = plan_to_rollout(&plan);
        let types: Vec<_> = rollout.stages.iter().map(|s| s.r#type).collect();
        let wp = types
            .iter()
            .position(|t| *t == RolloutStageType::DeployWaypoint)
            .expect("waypoint stage");
        let tr = types
            .iter()
            .position(|t| *t == RolloutStageType::TranslatePolicy)
            .expect("translate stage");
        assert!(wp < tr);
    }

    #[test]
    fn orders_canary_first() {
        let assessment = AssessmentResult {
            findings: vec![Finding {
                id: "readiness.vm-workload".into(),
                severity: FindingSeverity::Blocker,
                category: FindingCategory::Readiness,
                title: "vm".into(),
                message: "vm".into(),
                namespace: Some("blocked-ns".into()),
                resource: None,
                remediation: None,
                doc_url: None,
                evidence: None,
            }],
            scores: Default::default(),
            summary: Default::default(),
        };
        let waves = plan_waves(&["good-ns".into(), "blocked-ns".into()], &assessment);
        assert!(waves.iter().any(|w| w.name == "wave-1-canary"));
        assert!(waves.iter().any(|w| w.name == "wave-blocked"));
    }
}
