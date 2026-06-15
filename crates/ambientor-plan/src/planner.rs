use ambientor_core::inventory::AssessmentResult;

use crate::namespaces::namespaces_for_planning;
use ambientor_types::{
    AmbientAssessmentStatus, AssessmentScores, Finding, FindingSeverity, MeshTarget,
    MigrationPlanSpec, MigrationWave, PolicyTask, RolloutSpec, RolloutStage, RolloutStageType,
};

/// Namespaces per expand wave after the canary (keeps CRs and rollouts manageable at 10k+ scale).
pub const DEFAULT_WAVE_BATCH_SIZE: usize = 50;

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
    namespaces_for_planning(findings, &[])
}

/// Build an assessment result from a completed AmbientAssessment status.
pub fn assessment_result_from_status(status: &AmbientAssessmentStatus) -> AssessmentResult {
    assessment_result_from_status_and_findings(status, None)
}

/// Prefer `stored_findings` when CR status omits findings (Postgres canonical store).
pub fn assessment_result_from_status_and_findings(
    status: &AmbientAssessmentStatus,
    stored_findings: Option<Vec<Finding>>,
) -> AssessmentResult {
    let findings = stored_findings
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| status.findings.clone());
    AssessmentResult {
        findings,
        scores: AssessmentScores {
            readiness: status.readiness_score,
            sidecar_dependency: status.sidecar_dependency_score,
            traffic_compatibility: status.traffic_compatibility_score,
            overall: status.overall_score,
        },
        summary: status.summary.clone().unwrap_or_default(),
    }
}

/// Build a migration plan from assessment results (legacy: all namespaces from findings/inventory).
pub fn build_plan(assessment: &AssessmentResult, namespaces: &[String]) -> MigrationPlanSpec {
    let waves = plan_waves(namespaces, assessment);
    MigrationPlanSpec {
        assessment_ref: None,
        selected_namespaces: vec![],
        cluster_ref: None,
        display_name: None,
        target_mesh_mode: "ambient".into(),
        mesh_target: None,
        waves,
    }
}

/// Build a plan from an explicit namespace selection (preferred for large fleets).
pub fn build_plan_from_selection(
    selected_namespaces: &[String],
    mesh_target: Option<MeshTarget>,
    cluster_ref: Option<String>,
    display_name: Option<String>,
    assessment_ref: Option<String>,
    assessment: Option<&AssessmentResult>,
) -> MigrationPlanSpec {
    let mut ordered: Vec<String> = selected_namespaces
        .iter()
        .filter(|ns| !ns.is_empty())
        .cloned()
        .collect();
    ordered.sort();
    ordered.dedup();

    let empty = AssessmentResult {
        findings: vec![],
        scores: Default::default(),
        summary: Default::default(),
    };
    let assessment_for_waves = assessment.unwrap_or(&empty);

    let waves = plan_waves_batched(&ordered, assessment_for_waves, DEFAULT_WAVE_BATCH_SIZE);
    MigrationPlanSpec {
        assessment_ref,
        selected_namespaces: ordered,
        cluster_ref,
        display_name,
        target_mesh_mode: "ambient".into(),
        mesh_target,
        waves,
    }
}

/// Namespaces in `selected` that have blocker findings in `assessment`.
pub fn namespaces_with_blockers(
    selected: &[String],
    assessment: &AssessmentResult,
) -> Vec<String> {
    let blocker_ns: std::collections::HashSet<_> = assessment
        .findings
        .iter()
        .filter(|f| matches!(f.severity, FindingSeverity::Blocker))
        .filter_map(|f| f.namespace.clone())
        .collect();
    let mut blocked: Vec<_> = selected
        .iter()
        .filter(|ns| blocker_ns.contains(*ns))
        .cloned()
        .collect();
    blocked.sort();
    blocked
}

pub fn plan_waves(namespaces: &[String], assessment: &AssessmentResult) -> Vec<MigrationWave> {
    plan_waves_batched(namespaces, assessment, DEFAULT_WAVE_BATCH_SIZE)
}

/// Wave layout: canary → batched expand waves → optional blocked wave (not rolled out).
pub fn plan_waves_batched(
    namespaces: &[String],
    assessment: &AssessmentResult,
    batch_size: usize,
) -> Vec<MigrationWave> {
    let mut ordered: Vec<_> = namespaces.to_vec();
    ordered.sort();
    ordered.dedup();

    let blocker_ns: std::collections::HashSet<_> = assessment
        .findings
        .iter()
        .filter(|f| matches!(f.severity, FindingSeverity::Blocker))
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

    let batch_size = batch_size.max(1);
    let mut waves = Vec::new();
    if !ready.is_empty() {
        waves.push(MigrationWave {
            name: "wave-1-canary".into(),
            namespaces: vec![ready[0].clone()],
            prerequisites: vec![
                "ztunnel DaemonSet healthy".into(),
                "istio-cni ambient mode enabled".into(),
            ],
            policy_tasks: vec![],
        });
        let rest = &ready[1..];
        for (i, chunk) in rest.chunks(batch_size).enumerate() {
            waves.push(MigrationWave {
                name: format!("wave-{}-expand", i + 2),
                namespaces: chunk.to_vec(),
                prerequisites: vec!["Previous wave verification passed".into()],
                policy_tasks: if i == 0 {
                    policy_tasks_for_namespaces(assessment, &ready)
                } else {
                    vec![]
                },
            });
        }
    }
    if !blocked.is_empty() {
        waves.push(MigrationWave {
            name: "wave-blocked".into(),
            namespaces: blocked,
            prerequisites: vec![
                "Resolve blocker findings (Istio migrate doc: What is not supported) before rollout"
                    .into(),
            ],
            policy_tasks: vec![],
        });
    }
    waves
}

fn policy_tasks_for_namespaces(
    assessment: &AssessmentResult,
    namespaces: &[String],
) -> Vec<PolicyTask> {
    let ns_set: std::collections::HashSet<_> = namespaces.iter().cloned().collect();
    assessment
        .findings
        .iter()
        .filter(|f| f.id.starts_with("traffic."))
        .filter(|f| {
            f.namespace
                .as_ref()
                .is_some_and(|ns| ns_set.contains(ns))
        })
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
        name: "approve-and-preflight".into(),
        r#type: RolloutStageType::DryRun,
        namespaces: vec![],
        requires_approval: true,
    }];

    for wave in plan.waves.iter() {
        if wave.name == "wave-blocked" {
            continue;
        }
        stages.push(RolloutStage {
            name: format!("{}-enroll", wave.name),
            r#type: RolloutStageType::EnrollNamespace,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
        });
        stages.push(RolloutStage {
            name: format!("{}-remove-injection", wave.name),
            r#type: RolloutStageType::RemoveInjection,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
        });
        stages.push(RolloutStage {
            name: format!("{}-restart-sidecars", wave.name),
            r#type: RolloutStageType::RollingRestart,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
        });
        stages.push(RolloutStage {
            name: format!("{}-label", wave.name),
            r#type: RolloutStageType::LabelNamespace,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
        });
        stages.push(RolloutStage {
            name: format!("{}-waypoint", wave.name),
            r#type: RolloutStageType::DeployWaypoint,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
        });
        stages.push(RolloutStage {
            name: format!("{}-translate", wave.name),
            r#type: RolloutStageType::TranslatePolicy,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
        });
        stages.push(RolloutStage {
            name: format!("{}-restart", wave.name),
            r#type: RolloutStageType::RollingRestart,
            namespaces: wave.namespaces.clone(),
            requires_approval: false,
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
        cluster_ref: plan.cluster_ref.clone(),
        auto_rollback: true,
        mesh_target: plan.mesh_target.clone(),
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
    fn batched_waves_for_large_selection() {
        let ns: Vec<String> = (0..120).map(|i| format!("app-{i}")).collect();
        let empty = AssessmentResult {
            findings: vec![],
            scores: Default::default(),
            summary: Default::default(),
        };
        let waves = plan_waves_batched(&ns, &empty, 50);
        assert!(waves.iter().any(|w| w.name == "wave-1-canary"));
        let expand: Vec<_> = waves
            .iter()
            .filter(|w| w.name.starts_with("wave-") && w.name.contains("expand"))
            .collect();
        assert!(expand.len() >= 3);
    }

    #[test]
    fn selection_rejects_blockers_in_helper() {
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
        let blocked = namespaces_with_blockers(&["blocked-ns".into(), "ok-ns".into()], &assessment);
        assert_eq!(blocked, vec!["blocked-ns"]);
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
