use ambientor_types::dto::Finding;

use crate::rules::{RuleContext, RuleRegistry};
use crate::scoring::compute_scores;

/// Result of a full mesh assessment pipeline.
#[derive(Clone, Debug)]
pub struct AssessmentResult {
    pub findings: Vec<Finding>,
    pub scores: ambientor_types::AssessmentScores,
    pub summary: ambientor_types::FindingSummary,
}

pub fn run_assessment(registry: &RuleRegistry, ctx: &RuleContext) -> AssessmentResult {
    let findings = registry.evaluate_all(ctx);
    let scores = compute_scores(&findings);
    let summary = ambientor_types::FindingSummary::from_findings(&findings);
    AssessmentResult {
        findings,
        scores,
        summary,
    }
}
