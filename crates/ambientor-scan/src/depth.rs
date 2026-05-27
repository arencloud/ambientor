use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

const ISTIO_AMBIENT_MIGRATE: &str = "https://preliminary.istio.io/latest/docs/ambient/migrate/";

/// Minimum Istio version for ambient mesh (see project README).
const MIN_MAJOR: u32 = 1;
const MIN_MINOR: u32 = 24;

pub fn parse_istio_major_minor(version: &str) -> Option<(u32, u32)> {
    let trimmed = version.trim();
    let mut parts = trimmed.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

pub fn istio_version_sufficient(version: &str) -> bool {
    parse_istio_major_minor(version)
        .is_some_and(|(maj, min)| maj > MIN_MAJOR || (maj == MIN_MAJOR && min >= MIN_MINOR))
}

pub struct IstioVersionGateRule;

impl Rule for IstioVersionGateRule {
    fn id(&self) -> RuleId {
        "readiness.istio-version"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        let Some(ref version) = ctx.mesh_version else {
            return vec![{
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "Istio control-plane version unknown",
                    "Could not detect istiod version; confirm Istio is 1.24+ before ambient migration.",
                );
                f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
                f
            }];
        };
        if istio_version_sufficient(version) {
            return vec![];
        }
        // Legacy OSSM placeholder (no longer set); do not treat as semver failure.
        if version.starts_with("ossm-")
            && ctx
                .mesh_flavor
                .as_deref()
                .is_some_and(|f| f.contains("OSSM"))
        {
            return vec![{
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "Istio control-plane version unknown",
                    format!(
                        "OpenShift Service Mesh is present but istiod semver was not detected (reported '{version}'); confirm Istio 1.24+ from istiod image or istio.io/rev label."
                    ),
                );
                f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
                f.evidence = Some(format!("meshVersion: {version}"));
                f
            }];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Blocker,
                self.category(),
                "Istio version below ambient minimum",
                format!(
                    "Detected Istio version '{version}'; ambient migration requires Istio {MIN_MAJOR}.{MIN_MINOR} or newer."
                ),
            );
            f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
            f.remediation = Some(format!(
                "Upgrade Istio to {MIN_MAJOR}.{MIN_MINOR}+ with the ambient profile before migrating workloads"
            ));
            f.evidence = Some(format!("meshVersion: {version}"));
            f
        }]
    }
}

pub struct SpireWorkloadRule;

impl Rule for SpireWorkloadRule {
    fn id(&self) -> RuleId {
        "readiness.spire-workloads"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        if !ctx.platform.spire_detected {
            return vec![];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Blocker,
                self.category(),
                "SPIRE / SPIFFE workloads detected",
                "SPIRE-based workload identity is not supported with Istio ambient mesh; remove SPIRE before migrating.",
            );
            f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
            f.remediation = Some(
                "Migrate off SPIRE/SPIFFE identity or remain on sidecar mode until supported"
                    .into(),
            );
            f.evidence = Some(ctx.platform.spire_hits.join("\n"));
            f
        }]
    }
}

pub struct EnvoyFilterWaypointRule;

impl Rule for EnvoyFilterWaypointRule {
    fn id(&self) -> RuleId {
        "readiness.envoyfilter-waypoint"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.policies
            .envoy_filters_waypoint
            .iter()
            .map(|name| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Blocker,
                    self.category(),
                    "EnvoyFilter targets waypoint proxy",
                    format!(
                        "EnvoyFilter '{name}' applies to waypoint or gateway proxies; EnvoyFilter is not supported on waypoints in ambient mode."
                    ),
                );
                f.resource = Some(name.clone());
                f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
                f.remediation = Some(
                    "Remove or replace EnvoyFilter with supported extension mechanisms (e.g. WasmPlugin) before migration"
                        .into(),
                );
                f.evidence = Some(format!(
                    "resource: {name}\nspec: waypoint/gateway selector or proxyType detected"
                ));
                f
            })
            .collect()
    }
}

pub struct OssmMemberRollRule;

impl Rule for OssmMemberRollRule {
    fn id(&self) -> RuleId {
        "platform.ossm-member-roll"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Platform
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        let is_ossm = ctx
            .mesh_flavor
            .as_deref()
            .is_some_and(|f| f.contains("OSSM"));
        if !is_ossm || ctx.platform.ossm_member_namespaces.is_empty() {
            return vec![];
        }
        let members: std::collections::HashSet<_> = ctx
            .platform
            .ossm_member_namespaces
            .iter()
            .cloned()
            .collect();

        ctx.namespaces
            .iter()
            .filter(|ns| ns.injection_enabled || ns.ambient_enabled)
            .filter(|ns| !members.contains(&ns.name))
            .map(|ns| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "Namespace not enrolled in ServiceMeshMemberRoll",
                    format!(
                        "Namespace '{}' has mesh workloads but is not listed in a ServiceMeshMemberRoll member set.",
                        ns.name
                    ),
                );
                f.namespace = Some(ns.name.clone());
                f.remediation = Some(
                    "Add namespace to a ServiceMeshMemberRoll or enroll via OSSM console".into(),
                );
                f.evidence = Some(format!(
                    "namespace: {}\nmemberRollNamespaces: {:?}",
                    ns.name, ctx.platform.ossm_member_namespaces
                ));
                f
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_gate_accepts_1_30() {
        assert!(istio_version_sufficient("1.30.0"));
        assert!(!istio_version_sufficient("1.23.5"));
    }
}
