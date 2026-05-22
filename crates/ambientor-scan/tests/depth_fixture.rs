use ambientor_core::rules::RuleContext;
use ambientor_scan::default_registry;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    #[serde(flatten)]
    ctx: RuleContext,
}

#[test]
fn depth_rules_fire_from_fixture() {
    let data = include_str!("fixtures/depth_rules.json");
    let fixture: Fixture = serde_json::from_str(data).expect("parse");
    let findings = default_registry().evaluate_all(&fixture.ctx);
    assert!(
        findings
            .iter()
            .any(|f| f.id == "readiness.envoyfilter-waypoint"),
        "expected waypoint EnvoyFilter blocker"
    );
    assert!(
        findings.iter().any(|f| f.id == "readiness.spire-workloads"),
        "expected SPIRE blocker"
    );
}
