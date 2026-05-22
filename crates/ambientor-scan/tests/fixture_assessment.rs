use ambientor_core::rules::RuleContext;
use ambientor_scan::default_registry;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    #[serde(flatten)]
    ctx: RuleContext,
}

#[test]
fn mixed_mode_fixture_produces_traffic_warning() {
    let data = include_str!("fixtures/mixed_mode.json");
    let fixture: Fixture = serde_json::from_str(data).expect("parse fixture");
    let findings = default_registry().evaluate_all(&fixture.ctx);
    assert!(
        findings
            .iter()
            .any(|f| f.id == "traffic.mixed-mode-l7-bypass"),
        "expected mixed-mode finding"
    );
}
