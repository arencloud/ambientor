use ambientor_types::{AssessmentScores, Finding, FindingSeverity};

/// Compute category scores (0-100) from findings. Blockers cap the score.
pub fn compute_scores(findings: &[Finding]) -> AssessmentScores {
    let readiness = score_category(findings, |f| {
        matches!(
            f.category,
            ambientor_types::FindingCategory::Readiness
                | ambientor_types::FindingCategory::Platform
        )
    });
    let sidecar = score_category(findings, |f| {
        matches!(
            f.category,
            ambientor_types::FindingCategory::SidecarDependency
        )
    });
    let traffic = score_category(findings, |f| {
        matches!(
            f.category,
            ambientor_types::FindingCategory::TrafficCompatibility
        )
    });
    let overall = (readiness as u16 + sidecar as u16 + traffic as u16) / 3;

    AssessmentScores {
        readiness,
        sidecar_dependency: sidecar,
        traffic_compatibility: traffic,
        overall: overall as u8,
    }
}

fn score_category(findings: &[Finding], pred: impl Fn(&Finding) -> bool) -> u8 {
    let relevant: Vec<_> = findings.iter().filter(|f| pred(f)).collect();
    if relevant.is_empty() {
        return 100;
    }
    let mut score: i32 = 100;
    for f in relevant {
        match f.severity {
            FindingSeverity::Blocker => score = score.saturating_sub(40),
            FindingSeverity::Warning => score = score.saturating_sub(15),
            FindingSeverity::Info => score = score.saturating_sub(5),
        }
    }
    score.clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_types::{FindingCategory, FindingSeverity};

    #[test]
    fn blocker_reduces_score() {
        let findings = vec![Finding {
            id: "t".into(),
            severity: FindingSeverity::Blocker,
            category: FindingCategory::Readiness,
            title: "x".into(),
            message: "y".into(),
            namespace: None,
            resource: None,
            remediation: None,
            doc_url: None,
            evidence: None,
        }];
        let s = compute_scores(&findings);
        assert!(s.readiness < 100);
    }
}
