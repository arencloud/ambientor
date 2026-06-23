//! Portal assessment via `AmbientAssessment` CR (operator reconcile path).

use std::time::{Duration, Instant};

use ambientor_types::{
    AmbientAssessment, AmbientAssessmentSpec, AmbientAssessmentStatus, FindingSummary,
};
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use tracing::info;

const FIELD_MANAGER: &str = "ambientor-api";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_WAIT: Duration = Duration::from_secs(600);

/// Install namespace for portal-created assessment CRs.
pub fn install_namespace() -> String {
    std::env::var("AMBIENTOR_NAMESPACE").unwrap_or_else(|_| "ambientor-system".into())
}

/// When true, skip CR creation and use inline assessment in the API (dev fallback).
pub fn direct_assess_enabled() -> bool {
    matches!(
        std::env::var("AMBIENTOR_ASSESS_DIRECT").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

pub struct CompletedAssessment {
    pub name: String,
    pub namespace: String,
    pub status: AmbientAssessmentStatus,
}

/// Create `AmbientAssessment` and wait until the operator marks it `Completed`.
pub async fn trigger_and_wait(
    client: &Client,
    cluster_ref: &str,
) -> Result<CompletedAssessment, String> {
    let ns = install_namespace();
    let name = format!("portal-assessment-{}", chrono::Utc::now().timestamp());

    let assessment = AmbientAssessment::new(
        &name,
        AmbientAssessmentSpec {
            inventory_ref: None,
            cluster_ref: Some(cluster_ref.to_string()),
        },
    );

    let api: Api<AmbientAssessment> = Api::namespaced(client.clone(), &ns);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(&name, &pp, &Patch::Apply(&assessment))
        .await
        .map_err(|e| format!("create AmbientAssessment: {e}"))?;

    let pending_status = serde_json::json!({
        "status": {
            "phase": "Pending",
            "readinessScore": 0,
            "sidecarDependencyScore": 0,
            "trafficCompatibilityScore": 0,
            "overallScore": 0,
            "findings": [],
            "summary": { "blockers": 0, "warnings": 0, "info": 0 }
        }
    });
    api.patch_status(&name, &Default::default(), &Patch::Merge(pending_status))
        .await
        .map_err(|e| format!("patch AmbientAssessment status: {e}"))?;

    info!(
        assessment = %name,
        namespace = %ns,
        cluster_ref = %cluster_ref,
        "portal assessment CR created; waiting for operator"
    );

    let deadline = Instant::now() + MAX_WAIT;
    loop {
        let obj = api
            .get(&name)
            .await
            .map_err(|e| format!("get AmbientAssessment: {e}"))?;
        if let Some(status) = obj.status.clone() {
            match status.phase.as_str() {
                "Completed" => {
                    return Ok(CompletedAssessment {
                        name,
                        namespace: ns,
                        status,
                    });
                }
                "Failed" => {
                    return Err(format!(
                        "AmbientAssessment {ns}/{name} failed: {:?}",
                        status.summary
                    ));
                }
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out after {}s waiting for AmbientAssessment {ns}/{name} to complete",
                MAX_WAIT.as_secs()
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

impl CompletedAssessment {
    pub fn scores(&self) -> ambientor_types::AssessmentScores {
        ambientor_types::AssessmentScores {
            readiness: self.status.readiness_score,
            sidecar_dependency: self.status.sidecar_dependency_score,
            traffic_compatibility: self.status.traffic_compatibility_score,
            overall: self.status.overall_score,
        }
    }

    pub fn summary(&self) -> FindingSummary {
        self.status
            .summary
            .clone()
            .unwrap_or_else(|| FindingSummary::from_findings(&self.status.findings))
    }
}
