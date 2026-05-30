use ambientor_core::migrate_doc::{MIGRATE_DOC, NOT_SUPPORTED_SECTION};
use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

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
            return vec![version_unknown_finding(
                "Could not detect istiod version. Ambient requires Istio 1.24+; confirm the control plane image or `istio.io/rev` labels before migrating workloads.",
            )];
        };
        if istio_version_sufficient(version) {
            return vec![];
        }
        if version.starts_with("ossm-")
            && ctx
                .mesh_flavor
                .as_deref()
                .is_some_and(|f| f.contains("OSSM"))
        {
            return vec![version_unknown_finding(&format!(
                "OpenShift Service Mesh is present but istiod semver was not detected (reported '{version}'). Confirm Istio 1.24+ from the Sail/istiod deployment before ambient migration."
            ))];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Warning,
                self.category(),
                "Istio version below ambient minimum",
                format!(
                    "Detected Istio version '{version}'. Ambient mode requires Istio {MIN_MAJOR}.{MIN_MINOR} or newer. \
                     This is a platform readiness gap (not listed under \"What is not supported\"), but migration should not proceed until upgraded."
                ),
            );
            f.doc_url = Some(MIGRATE_DOC.into());
            f.remediation = Some(format!(
                "1. Upgrade the control plane to Istio {MIN_MAJOR}.{MIN_MINOR}+ with the ambient profile (ztunnel + CNI).\n\
                 2. Verify `istioctl version` and istiod image tags match the target release.\n\
                 3. Re-run assessment after upgrade before labeling application namespaces."
            ));
            f.evidence = Some(format!("meshVersion: {version}"));
            f
        }]
    }
}

fn version_unknown_finding(message: &str) -> Finding {
    let mut f = finding(
        "readiness.istio-version",
        FindingSeverity::Warning,
        FindingCategory::Readiness,
        "Istio control-plane version unknown",
        message,
    );
    f.doc_url = Some(MIGRATE_DOC.into());
    f.remediation = Some(
        "1. Inspect istiod deployment images and `istio.io/rev` labels on istiod pods.\n\
         2. Confirm semver is at least 1.24.\n\
         3. Re-run assessment once version is visible to Ambientor."
            .into(),
    );
    f
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
                "SPIRE / SPIFFE identity is not supported in ambient mode",
                "Istio ambient migration docs list SPIRE as a certificate provider under \
                 \"What is not supported\" — hard blockers. Workloads using SPIRE-based identity \
                 cannot join the ambient mesh until SPIRE is removed or replaced.",
            );
            f.doc_url = Some(NOT_SUPPORTED_SECTION.into());
            f.remediation = Some(
                "1. Inventory all SPIRE/SPIFFE agents and registrations touching mesh workloads.\n\
                 2. Choose a path: remain on sidecar mode, or migrate workloads to Istio-managed certificates only.\n\
                 3. Remove SPIRE integration from enrolled namespaces before ambient enrollment.\n\
                 4. Re-run assessment with zero SPIRE hits before proceeding."
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
                    FindingSeverity::Warning,
                    self.category(),
                    "EnvoyFilter cannot be applied to waypoint proxies",
                    format!(
                        "EnvoyFilter `{name}` targets waypoint or gateway proxies. The migrate guide \
                         lists EnvoyFilter on waypoints under known limitations — configurations \
                         cannot be carried over; use WasmPlugin or other supported extensions instead."
                    ),
                );
                f.resource = Some(name.clone());
                f.doc_url = Some(MIGRATE_DOC.into());
                f.remediation = Some(
                    "1. Capture the EnvoyFilter patch intent (Lua, metadata, custom filters).\n\
                     2. Evaluate WasmPlugin attached to the waypoint via `targetRefs` where supported.\n\
                     3. Remove the waypoint/gateway-scoped EnvoyFilter before enabling ambient on affected namespaces.\n\
                     4. Validate L7 behavior in staging after replacement."
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
                        "Namespace '{}' has mesh workloads but is not listed in a ServiceMeshMemberRoll member set on OpenShift Service Mesh.",
                        ns.name
                    ),
                );
                f.namespace = Some(ns.name.clone());
                f.doc_url = Some(MIGRATE_DOC.into());
                f.remediation = Some(
                    "1. Open the ServiceMeshMemberRoll for your control plane revision.\n\
                     2. Add this namespace to `spec.members` or use the OSSM console enrollment flow.\n\
                     3. Confirm istiod discovers the namespace before ambient labeling.\n\
                     4. Re-run assessment."
                        .into(),
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
