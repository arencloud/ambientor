use ambientor_core::rules::RuleContext;
use ambientor_scan::default_registry;
use ambientor_types::FindingSeverity;
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
    let envoy = findings
        .iter()
        .find(|f| f.id == "readiness.envoyfilter-waypoint")
        .expect("expected waypoint EnvoyFilter finding");
    assert_eq!(
        envoy.severity,
        FindingSeverity::Warning,
        "EnvoyFilter on waypoint is a known limitation, not a hard blocker"
    );
    let spire = findings
        .iter()
        .find(|f| f.id == "readiness.spire-workloads")
        .expect("expected SPIRE finding");
    assert_eq!(
        spire.severity,
        FindingSeverity::Blocker,
        "SPIRE is under What is not supported"
    );
    assert!(envoy.remediation.is_some());
    assert!(spire.remediation.is_some());
}
