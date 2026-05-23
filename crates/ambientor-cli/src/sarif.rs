//! SARIF 2.1.0 export for assessment findings (CI / security scanners).

use ambientor_types::{Finding, FindingCategory, FindingSeverity};
use serde::Serialize;
use serde_json::{Value, json};

use crate::AssessOutput;

const SARIF_SCHEMA: &str = "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json";
const TOOL_URI: &str = "https://github.com/arencloud/ambientor";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLog {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDriver {
    name: &'static str,
    version: String,
    information_uri: &'static str,
    rules: Vec<SarifRule>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRule {
    id: String,
    short_description: SarifMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    help_uri: Option<String>,
    default_configuration: SarifRuleConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRuleConfig {
    level: &'static str,
}

#[derive(Serialize)]
struct SarifMessage {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult {
    rule_id: String,
    level: &'static str,
    message: SarifMessage,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    locations: Vec<SarifLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation {
    physical_location: SarifPhysicalLocation,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_location: Option<SarifArtifactLocation>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifArtifactLocation {
    uri: String,
}

pub fn to_sarif(out: &AssessOutput) -> Value {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let rules = build_rules(&out.findings);
    let results = out
        .findings
        .iter()
        .map(finding_to_result)
        .collect::<Vec<_>>();

    let log = SarifLog {
        schema: SARIF_SCHEMA,
        version: "2.1.0",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "ambientor",
                    version,
                    information_uri: TOOL_URI,
                    rules,
                },
            },
            results,
            properties: Some(json!({
                "scores": out.scores,
                "summary": out.summary,
            })),
        }],
    };
    serde_json::to_value(log).expect("SARIF serializes")
}

fn build_rules(findings: &[Finding]) -> Vec<SarifRule> {
    let mut rules = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in findings {
        if !seen.insert(f.id.clone()) {
            continue;
        }
        rules.push(SarifRule {
            id: f.id.clone(),
            short_description: SarifMessage {
                text: f.title.clone(),
            },
            help_uri: f.doc_url.clone(),
            default_configuration: SarifRuleConfig {
                level: severity_level(f.severity),
            },
        });
    }
    rules
}

fn finding_to_result(f: &Finding) -> SarifResult {
    let mut props = serde_json::Map::new();
    props.insert("category".into(), json!(category_str(f.category)));
    if let Some(r) = &f.remediation {
        props.insert("remediation".into(), json!(r));
    }
    if let Some(e) = &f.evidence {
        props.insert("evidence".into(), json!(e));
    }
    if let Some(ns) = &f.namespace {
        props.insert("namespace".into(), json!(ns));
    }

    let locations = f
        .resource
        .as_ref()
        .map(|r| {
            vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: Some(SarifArtifactLocation { uri: r.clone() }),
                },
            }]
        })
        .unwrap_or_default();

    SarifResult {
        rule_id: f.id.clone(),
        level: severity_level(f.severity),
        message: SarifMessage {
            text: format!("{} — {}", f.title, f.message),
        },
        locations,
        properties: if props.is_empty() {
            None
        } else {
            Some(Value::Object(props))
        },
    }
}

fn category_str(c: FindingCategory) -> &'static str {
    match c {
        FindingCategory::Readiness => "readiness",
        FindingCategory::SidecarDependency => "sidecar_dependency",
        FindingCategory::TrafficCompatibility => "traffic_compatibility",
        FindingCategory::PolicyTranslation => "policy_translation",
        FindingCategory::Platform => "platform",
    }
}

fn severity_level(sev: FindingSeverity) -> &'static str {
    match sev {
        FindingSeverity::Blocker => "error",
        FindingSeverity::Warning => "warning",
        FindingSeverity::Info => "note",
    }
}

#[cfg(test)]
mod tests {
    use ambientor_types::{FindingCategory, FindingSeverity};

    use super::*;
    use crate::AssessOutput;

    #[test]
    fn sarif_includes_evidence_and_scores() {
        let out = AssessOutput {
            findings: vec![Finding {
                id: "test.rule".into(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::TrafficCompatibility,
                title: "Test rule".into(),
                message: "Something is wrong".into(),
                namespace: Some("bookinfo".into()),
                resource: Some("DestinationRule/reviews".into()),
                remediation: Some("Migrate subsets".into()),
                doc_url: Some("https://istio.io/latest/docs/".into()),
                evidence: Some("subsets:\n  - v1".into()),
            }],
            scores: ambientor_types::AssessmentScores {
                overall: 80,
                readiness: 90,
                sidecar_dependency: 85,
                traffic_compatibility: 70,
            },
            summary: ambientor_types::FindingSummary {
                blockers: 0,
                warnings: 1,
                info: 0,
            },
        };
        let v = to_sarif(&out);
        assert_eq!(v["version"], "2.1.0");
        let result = &v["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "test.rule");
        assert_eq!(result["level"], "warning");
        assert_eq!(result["properties"]["evidence"], "subsets:\n  - v1");
        assert_eq!(v["runs"][0]["properties"]["scores"]["overall"], 80);
    }

    #[test]
    fn blocker_maps_to_error_level() {
        let out = AssessOutput {
            findings: vec![Finding {
                id: "block".into(),
                severity: FindingSeverity::Blocker,
                category: FindingCategory::Readiness,
                title: "Blocked".into(),
                message: "Cannot migrate".into(),
                namespace: None,
                resource: None,
                remediation: None,
                doc_url: None,
                evidence: None,
            }],
            scores: Default::default(),
            summary: Default::default(),
        };
        let v = to_sarif(&out);
        assert_eq!(v["runs"][0]["results"][0]["level"], "error");
    }
}
