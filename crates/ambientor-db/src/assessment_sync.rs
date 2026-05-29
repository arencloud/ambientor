use ambientor_core::rules::RuleContext;
use ambientor_dashboard::{
    build_cluster_assessment_from_context, cluster_dashboard_meta, dashboard_from_assessment_run,
};
use ambientor_types::Finding;
use kube::Client;

use crate::pool::DbError;
use crate::traits::{ApplicationAssessmentStore, DashboardStore};

/// Persist application rows and dashboard snapshot from one assessment pass.
pub async fn persist_full_assessment(
    applications: &dyn ApplicationAssessmentStore,
    dashboard: &dyn DashboardStore,
    client: &Client,
    cluster_ref: &str,
    ctx: &RuleContext,
    findings: &[Finding],
) -> Result<usize, DbError> {
    let run = build_cluster_assessment_from_context(client, cluster_ref, ctx, findings)
        .await
        .map_err(|e| DbError::Serialize(e.to_string()))?;

    let count = run.applications.len();
    applications.replace_run(&run).await?;

    let cluster_meta = cluster_dashboard_meta(client)
        .await
        .map_err(|e| DbError::Serialize(e.to_string()))?;
    let snapshot = dashboard_from_assessment_run(&run, cluster_meta);
    dashboard.sync_snapshot(&snapshot).await?;

    Ok(count)
}
