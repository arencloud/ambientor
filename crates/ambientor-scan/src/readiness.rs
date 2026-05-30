use ambientor_core::migrate_doc::{MIGRATE_DOC, NOT_SUPPORTED_SECTION};
use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

pub struct GatewayApiRule;

impl Rule for GatewayApiRule {
    fn id(&self) -> RuleId {
        "readiness.gateway-api"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        if ctx.gateway_api_present {
            return vec![];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Warning,
                self.category(),
                "Gateway API CRDs are not installed",
                "Ambient L7 routing and policy migration rely on Gateway API resources (HTTPRoute, Gateway). \
                 Without the standard-channel CRDs, you cannot migrate VirtualService-based routes cleanly.",
            );
            f.doc_url = Some(MIGRATE_DOC.into());
            f.remediation = Some(
                "1. Install Gateway API standard-channel CRDs in the cluster.\n\
                 2. Confirm `kubectl get crd httproutes.gateway.networking.k8s.io` succeeds.\n\
                 3. Proceed with policy migration (VirtualService → HTTPRoute) per the migrate guide.\n\
                 4. Re-run assessment."
                    .into(),
            );
            f
        }]
    }
}

pub struct AmbientComponentsRule;

impl Rule for AmbientComponentsRule {
    fn id(&self) -> RuleId {
        "readiness.ambient-components"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        if ctx.ambient_installed {
            return vec![];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Warning,
                self.category(),
                "Ambient data plane (ztunnel) not detected",
                "The cluster does not appear to have ambient components installed. The migrate guide \
                 requires ztunnel and an ambient-capable CNI before namespaces can join ambient mode.",
            );
            f.doc_url = Some(MIGRATE_DOC.into());
            f.remediation = Some(
                "1. Install or upgrade Istio with the ambient profile for your revision.\n\
                 2. Verify ztunnel DaemonSet pods are Running in the istio-system (or revision) namespace.\n\
                 3. Confirm istio-cni supports ambient redirection.\n\
                 4. Re-run assessment before migrating application namespaces."
                    .into(),
            );
            f
        }]
    }
}

pub struct PeerAuthDisableRule;

impl Rule for PeerAuthDisableRule {
    fn id(&self) -> RuleId {
        "readiness.peer-auth-disable"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.policies
            .peer_auth_disable
            .iter()
            .map(|name| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Blocker,
                    self.category(),
                    "PeerAuthentication with mode DISABLE cannot migrate",
                    format!(
                        "PeerAuthentication `{name}` sets `spec.mtls.mode: DISABLE`. Under \
                         \"What is not supported\", ambient always enforces mTLS between mesh workloads; \
                         DISABLE policies are ignored and block migration for affected scope."
                    ),
                );
                f.resource = Some(name.clone());
                f.namespace = resource_namespace(name);
                f.doc_url = Some(NOT_SUPPORTED_SECTION.into());
                f.remediation = Some(
                    "1. Identify why DISABLE was configured (legacy plaintext, debug, or tooling).\n\
                     2. Remove the PeerAuthentication or change mode to PERMISSIVE/STRICT with an explicit migration plan.\n\
                     3. Validate applications tolerate mTLS (use PERMISSIVE temporarily if needed).\n\
                     4. Re-run assessment with no DISABLE PeerAuthentication resources."
                        .into(),
                );
                f.evidence = Some(format!(
                    "resource: {name}\nspec.mtls.mode: DISABLE"
                ));
                f
            })
            .collect()
    }
}

pub struct VmWorkloadRule;

impl Rule for VmWorkloadRule {
    fn id(&self) -> RuleId {
        "readiness.vm-workload"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.namespaces
            .iter()
            .filter(|ns| ns.has_vm_workloads)
            .map(|ns| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Blocker,
                    self.category(),
                    "VM workloads cannot join the ambient mesh",
                    format!(
                        "Namespace '{}' contains VM or bare-metal workloads registered in the mesh. \
                         Istio documents VM workloads under \"What is not supported\" as a hard blocker — \
                         they cannot join ambient mode with current guidance.",
                        ns.name
                    ),
                );
                f.namespace = Some(ns.name.clone());
                f.doc_url = Some(NOT_SUPPORTED_SECTION.into());
                f.remediation = Some(
                    "1. List WorkloadEntry / VM instances in this namespace.\n\
                     2. Exclude the namespace from ambient migration scope, or retire VM-based mesh participation.\n\
                     3. If Istio publishes VM ambient onboarding for your version, follow that guide explicitly.\n\
                     4. Re-run assessment only when no VM workloads remain on the migration path."
                        .into(),
                );
                f.evidence = Some(format!(
                    "namespace: {}\nworkloadCount: {}",
                    ns.name, ns.workload_count
                ));
                f
            })
            .collect()
    }
}

fn resource_namespace(resource: &str) -> Option<String> {
    let (ns, _) = resource.split_once('/')?;
    Some(ns.to_string())
}
