use ambientor_k8s::PlatformInfo;
use k8s_openapi::api::core::v1::Namespace;
use kube::{Api, Client};
use serde::{Deserialize, Serialize};

use crate::dynamic::{api_resource, list_cluster_cr, list_cr_in_namespace, list_namespaced_cr};
use crate::platform_scan::collect_ossm_member_namespaces;

/// SCC names commonly required for mesh control plane and waypoint workloads.
const PREFERRED_SCC: &[&str] = &["anyuid", "privileged", "istio-cni", "istio-ingressgateway"];

const OLM_WATCH_NAMESPACES: &[&str] = &[
    "openshift-operators",
    "openshift-marketplace",
    "istio-system",
    "openshift-servicemesh",
];

const OPERATOR_NAME_HINTS: &[&str] = &[
    "servicemesh",
    "maistra",
    "istio",
    "ossm",
    "sail",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenShiftWizardOptions {
    /// Namespace where ambientor operator runs (for SCC check).
    pub ambientor_namespace: String,
    /// Operator ServiceAccount name.
    pub operator_service_account: String,
    /// Namespaces to suggest adding to ServiceMeshMemberRoll.
    #[serde(default)]
    pub enroll_namespaces: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenShiftWizardReport {
    pub is_openshift: bool,
    pub mesh_flavor: String,
    pub steps: Vec<WizardStep>,
    pub member_roll: MemberRollWizard,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardStep {
    pub id: String,
    pub title: String,
    pub passed: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberRollWizard {
    pub existing_members: Vec<String>,
    pub missing_enrollments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_manifest: Option<String>,
}

pub async fn run_wizard(
    client: &Client,
    platform: &PlatformInfo,
    opts: &OpenShiftWizardOptions,
) -> anyhow::Result<OpenShiftWizardReport> {
    let mut steps = Vec::new();

    if !platform.is_openshift {
        steps.push(WizardStep {
            id: "openshift-platform".into(),
            title: "OpenShift cluster".into(),
            passed: false,
            message: "Cluster does not appear to be OpenShift (no routes.openshift.io CRD)".into(),
            remediation: Some(
                "Run this wizard on an OpenShift or OSSM cluster, or use upstream Istio preflight"
                    .into(),
            ),
            details: None,
        });
        return Ok(OpenShiftWizardReport {
            is_openshift: false,
            mesh_flavor: format!("{:?}", platform.mesh_flavor),
            steps,
            member_roll: MemberRollWizard {
                existing_members: vec![],
                missing_enrollments: opts.enroll_namespaces.clone(),
                suggested_manifest: None,
            },
        });
    }

    steps.push(WizardStep {
        id: "openshift-platform".into(),
        title: "OpenShift cluster".into(),
        passed: true,
        message: "OpenShift API surface detected".into(),
        remediation: None,
        details: None,
    });

    steps.push(check_olm_operator(client).await);
    steps.push(check_operator_scc(
        client,
        &opts.ambientor_namespace,
        &opts.operator_service_account,
    )
    .await);

    let member_roll = member_roll_wizard(client, opts).await;
    steps.push(member_roll_step(&member_roll));

    Ok(OpenShiftWizardReport {
        is_openshift: true,
        mesh_flavor: format!("{:?}", platform.mesh_flavor),
        steps,
        member_roll,
    })
}

async fn check_olm_operator(client: &Client) -> WizardStep {
    let sub_ar = api_resource(
        "operators.coreos.com",
        "v1alpha1",
        "Subscription",
        "subscriptions",
    );
    let csv_ar = api_resource(
        "operators.coreos.com",
        "v1alpha1",
        "ClusterServiceVersion",
        "clusterserviceversions",
    );

    let mut subscriptions = Vec::new();
    for ns in OLM_WATCH_NAMESPACES {
        if let Ok(items) = list_cr_in_namespace(client, &sub_ar, ns).await {
            for item in items {
                let name = item.metadata.name.clone().unwrap_or_default();
                if operator_name_matches(&name) {
                    subscriptions.push(format!("{ns}/{name}"));
                }
            }
        }
    }

    let mut csv_ok = Vec::new();
    if let Ok(csvs) = list_namespaced_cr(client, &csv_ar).await {
        for csv in csvs {
            let name = csv.metadata.name.clone().unwrap_or_default();
            if !operator_name_matches(&name) {
                continue;
            }
            let phase = csv
                .data
                .get("status")
                .and_then(|s| s.get("phase"))
                .and_then(|p| p.as_str())
                .unwrap_or("Unknown");
            if phase == "Succeeded" {
                csv_ok.push(name);
            }
        }
    }

    let passed = !subscriptions.is_empty() || !csv_ok.is_empty();
    WizardStep {
        id: "olm-servicemesh-operator".into(),
        title: "OLM Service Mesh operator".into(),
        passed,
        message: if passed {
            format!(
                "Found mesh operator via OLM (subscriptions: {}, succeeded CSVs: {})",
                subscriptions.len(),
                csv_ok.len()
            )
        } else {
            "No Service Mesh / Maistra OLM Subscription or succeeded CSV detected".into()
        },
        remediation: Some(
            "Install OpenShift Service Mesh operator from OperatorHub (OLM) before migration"
                .into(),
        ),
        details: Some(serde_json::json!({
            "subscriptions": subscriptions,
            "succeededCsv": csv_ok,
        })),
    }
}

async fn check_operator_scc(client: &Client, namespace: &str, sa: &str) -> WizardStep {
    let ar = api_resource(
        "security.openshift.io",
        "v1",
        "SecurityContextConstraints",
        "securitycontextconstraints",
    );
    let user = format!("system:serviceaccount:{namespace}:{sa}");
    let group = format!("system:serviceaccounts:{namespace}");

    let matched = match list_cluster_cr(client, &ar).await {
        Ok(sccs) => sccs
            .into_iter()
            .filter_map(|scc| {
                let name = scc.metadata.name.clone()?;
                let users = scc_users(&scc.data);
                let groups = scc_groups(&scc.data);
                if users.iter().any(|u| u == &user)
                    || groups.iter().any(|g| g == &group)
                    || users.iter().any(|u| u == "system:serviceaccounts")
                {
                    Some(name)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(),
        Err(e) => {
            return WizardStep {
                id: "openshift-scc".into(),
                title: "Operator SecurityContextConstraints".into(),
                passed: false,
                message: format!("Cannot list SCCs (not OpenShift or missing RBAC): {e}"),
                remediation: Some(
                    "Grant clusterrole to read security.openshift.io/securitycontextconstraints"
                        .into(),
                ),
                details: None,
            };
        }
    };

    let has_preferred = matched
        .iter()
        .any(|name| PREFERRED_SCC.contains(&name.as_str()));
    let passed = !matched.is_empty() && has_preferred;

    WizardStep {
        id: "openshift-scc".into(),
        title: "Operator SecurityContextConstraints".into(),
        passed,
        message: if matched.is_empty() {
            format!(
                "ServiceAccount '{namespace}/{sa}' is not bound to any SecurityContextConstraints"
            )
        } else if has_preferred {
            format!(
                "ServiceAccount '{namespace}/{sa}' may use SCC: {}",
                matched.join(", ")
            )
        } else {
            format!(
                "ServiceAccount '{namespace}/{sa}' uses SCC [{}] but none of {:?}; verify mesh compatibility",
                matched.join(", "),
                PREFERRED_SCC
            )
        },
        remediation: Some(
            "Bind ambientor-operator ServiceAccount to anyuid or a custom SCC allowed for mesh workloads"
                .into(),
        ),
        details: Some(serde_json::json!({
            "serviceAccount": format!("{namespace}/{sa}"),
            "matchedScc": matched,
            "preferred": PREFERRED_SCC,
        })),
    }
}

async fn member_roll_wizard(
    client: &Client,
    opts: &OpenShiftWizardOptions,
) -> MemberRollWizard {
    let existing = collect_ossm_member_namespaces(client).await;
    let missing: Vec<String> = opts
        .enroll_namespaces
        .iter()
        .filter(|ns| !existing.contains(ns))
        .cloned()
        .collect();

    let suggested_manifest = if missing.is_empty() && opts.enroll_namespaces.is_empty() {
        None
    } else {
        Some(suggest_member_roll_yaml(
            &existing,
            &opts.enroll_namespaces,
        ))
    };

    MemberRollWizard {
        existing_members: existing,
        missing_enrollments: missing,
        suggested_manifest,
    }
}

fn member_roll_step(roll: &MemberRollWizard) -> WizardStep {
    let has_roll = !roll.existing_members.is_empty();
    let needs_enroll = !roll.missing_enrollments.is_empty();
    let passed = has_roll && !needs_enroll;

    WizardStep {
        id: "ossm-member-roll".into(),
        title: "ServiceMeshMemberRoll enrollment".into(),
        passed,
        message: if !has_roll {
            "No ServiceMeshMemberRoll members found".into()
        } else if needs_enroll {
            format!(
                "Namespaces not enrolled: {}",
                roll.missing_enrollments.join(", ")
            )
        } else {
            format!(
                "All requested namespaces enrolled ({} members)",
                roll.existing_members.len()
            )
        },
        remediation: if needs_enroll {
            Some(
                "Apply suggested ServiceMeshMemberRoll manifest or enroll namespaces in OSSM console"
                    .into(),
            )
        } else if !has_roll {
            Some("Create ServiceMeshMemberRoll with target namespaces".into())
        } else {
            None
        },
        details: Some(serde_json::json!({
            "existingMembers": roll.existing_members,
            "missingEnrollments": roll.missing_enrollments,
        })),
    }
}

/// Merge existing MemberRoll members with namespaces to enroll and emit example YAML.
pub fn suggest_member_roll_yaml(existing: &[String], enroll: &[String]) -> String {
    let mut members: Vec<String> = existing.to_vec();
    for ns in enroll {
        if !members.contains(ns) {
            members.push(ns.clone());
        }
    }
    members.sort();
    members.dedup();

    let member_lines: String = members
        .iter()
        .map(|ns| format!("  - {ns}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"# Example ServiceMeshMemberRoll — review namespace and control plane namespace before apply
apiVersion: maistra.io/v1
kind: ServiceMeshMemberRoll
metadata:
  name: default
  namespace: istio-system
spec:
  members:
{member_lines}
"#
    )
}

fn operator_name_matches(name: &str) -> bool {
    let lower = name.to_lowercase();
    OPERATOR_NAME_HINTS
        .iter()
        .any(|hint| lower.contains(hint))
}

fn scc_users(data: &serde_json::Value) -> Vec<String> {
    data.get("users")
        .and_then(|u| u.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn scc_groups(data: &serde_json::Value) -> Vec<String> {
    data.get("groups")
        .and_then(|g| g.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Mesh namespaces with injection/ambient labels that are not yet in MemberRoll.
pub async fn namespaces_needing_enrollment(client: &Client) -> anyhow::Result<Vec<String>> {
    let members: std::collections::HashSet<String> =
        collect_ossm_member_namespaces(client).await.into_iter().collect();
    let ns_api: Api<Namespace> = Api::all(client.clone());
    let list = ns_api.list(&Default::default()).await?;
    let mut out = Vec::new();
    for ns in list.items {
        let Some(name) = ns.metadata.name else {
            continue;
        };
        if members.contains(&name) {
            continue;
        }
        let labels = ns.metadata.labels.as_ref();
        let mesh_ns = labels.is_some_and(|l| {
            l.get("istio-injection")
                .is_some_and(|v| v == "enabled" || v == "disabled")
                || l.get("istio.io/rev").is_some()
                || l.get("ambient.istio.io/waypoint")
                    .is_some_and(|v| !v.is_empty())
        });
        if mesh_ns {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_member_roll_merges_and_dedupes() {
        let yaml = suggest_member_roll_yaml(&["a".into(), "b".into()], &["b".into(), "c".into()]);
        assert!(yaml.contains("  - a"));
        assert!(yaml.contains("  - b"));
        assert!(yaml.contains("  - c"));
        assert!(yaml.matches("  - b").count() == 1);
    }
}
