use ambientor_core::rules::RuleContext;
use ambientor_scan::default_registry;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    #[serde(flatten)]
    ctx: RuleContext,
}

#[test]
fn sidecar_fixture_produces_workload_findings() {
    let data = include_str!("fixtures/sidecar_workloads.json");
    let fixture: Fixture = serde_json::from_str(data).expect("parse fixture");
    let findings = default_registry().evaluate_all(&fixture.ctx);
    assert!(
        findings.iter().any(|f| f.id == "sidecar.localhost-proxy"),
        "expected localhost proxy finding"
    );
    assert!(
        findings.iter().any(|f| f.id == "sidecar.hold-until-proxy"),
        "expected hold-until-proxy finding"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.id == "traffic.destination-rule-subsets"),
        "expected destination rule subsets finding"
    );
    assert!(
        findings.iter().all(|f| f.evidence.is_some()),
        "fixture findings should include evidence"
    );
}
