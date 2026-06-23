use ambientor_types::{AssessmentScores, Finding, FindingSummary};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::pool::DbError;
use crate::traits::ScanStore;

/// Cluster identifier for multi-cluster hubs (`AMBIENTOR_CLUSTER_REF`).
pub fn cluster_ref_from_env() -> String {
    std::env::var("AMBIENTOR_CLUSTER_REF").unwrap_or_else(|_| "in-cluster".into())
}

/// Assessment payload stored in `scan_runs.assessment_json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAssessment {
    pub findings: Vec<Finding>,
    pub scores: AssessmentScores,
    pub summary: FindingSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment_name: Option<String>,
}

#[derive(Clone, Debug, FromRow)]
pub struct ScanRunRow {
    pub id: Uuid,
    pub cluster_ref: Option<String>,
    pub namespace: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: String,
    pub assessment_json: serde_json::Value,
}

pub struct ScanRepository {
    pool: PgPool,
}

impl ScanRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn record_completed(
        &self,
        cluster_ref: &str,
        namespace: Option<&str>,
        payload: &StoredAssessment,
    ) -> Result<Uuid, DbError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let assessment_json =
            serde_json::to_value(payload).map_err(|e| DbError::Serialize(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO scan_runs
                (id, cluster_ref, namespace, started_at, finished_at, status, assessment_json)
            VALUES ($1, $2, $3, $4, $4, 'completed', $5)
            "#,
        )
        .bind(id)
        .bind(cluster_ref)
        .bind(namespace)
        .bind(now)
        .bind(assessment_json)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn list_recent(&self, limit: i64) -> Result<Vec<ScanRunRow>, DbError> {
        let rows = sqlx::query_as::<_, ScanRunRow>(
            r#"
            SELECT id, cluster_ref, namespace, started_at, finished_at, status, assessment_json
            FROM scan_runs
            ORDER BY COALESCE(finished_at, started_at) DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn latest_for_assessment(
        &self,
        cluster_ref: &str,
        assessment_name: &str,
    ) -> Result<Option<StoredAssessment>, DbError> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            r#"
            SELECT assessment_json
            FROM scan_runs
            WHERE cluster_ref = $1
              AND status = 'completed'
              AND assessment_json->>'assessmentName' = $2
            ORDER BY COALESCE(finished_at, started_at) DESC
            LIMIT 1
            "#,
        )
        .bind(cluster_ref)
        .bind(assessment_name)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|(json,)| {
            serde_json::from_value(json).map_err(|e| DbError::Serialize(e.to_string()))
        })
        .transpose()
    }
}

#[async_trait]
impl ScanStore for ScanRepository {
    async fn record_completed(
        &self,
        cluster_ref: &str,
        namespace: Option<&str>,
        payload: &StoredAssessment,
    ) -> Result<Uuid, DbError> {
        ScanRepository::record_completed(self, cluster_ref, namespace, payload).await
    }

    async fn list_recent(&self, limit: i64) -> Result<Vec<ScanRunRow>, DbError> {
        ScanRepository::list_recent(self, limit).await
    }

    async fn latest_for_assessment(
        &self,
        cluster_ref: &str,
        assessment_name: &str,
    ) -> Result<Option<StoredAssessment>, DbError> {
        ScanRepository::latest_for_assessment(self, cluster_ref, assessment_name).await
    }
}

/// Load Postgres findings for assessments whose CR status omits them.
pub async fn load_assessment_findings_overrides(
    scan_repo: &dyn ScanStore,
    cluster_ref: &str,
    assessment_names: &[String],
) -> Result<std::collections::HashMap<String, Vec<Finding>>, DbError> {
    let mut out = std::collections::HashMap::new();
    for name in assessment_names {
        if let Some(stored) = scan_repo.latest_for_assessment(cluster_ref, name).await?
            && !stored.findings.is_empty()
        {
            out.insert(name.clone(), stored.findings);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use ambientor_types::FindingSeverity;

    use super::*;

    #[test]
    fn stored_assessment_roundtrip() {
        let payload = StoredAssessment {
            findings: vec![Finding {
                id: "test".into(),
                severity: FindingSeverity::Info,
                category: ambientor_types::FindingCategory::Readiness,
                title: "t".into(),
                message: "m".into(),
                namespace: None,
                resource: None,
                remediation: None,
                doc_url: None,
                evidence: None,
            }],
            scores: AssessmentScores::default(),
            summary: FindingSummary::default(),
            source: Some("api".into()),
            assessment_name: None,
        };
        let json = serde_json::to_value(&payload).unwrap();
        let back: StoredAssessment = serde_json::from_value(json).unwrap();
        assert_eq!(back.findings.len(), 1);
        assert_eq!(back.source.as_deref(), Some("api"));
    }
}
