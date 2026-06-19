//! Spoke cluster RBAC checks for hub-driven assess and rollout.

use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
};
use kube::{Api, Client};

use crate::remote::RemoteClientError;

/// One API permission probed via SelfSubjectAccessReview.
#[derive(Clone, Copy, Debug)]
struct AccessCheck {
    group: &'static str,
    resource: &'static str,
    verb: &'static str,
    /// Shown in operator / portal messages when denied.
    label: &'static str,
}

const ROLLOUT_CHECKS: &[AccessCheck] = &[
    AccessCheck {
        group: "",
        resource: "namespaces",
        verb: "patch",
        label: "namespaces patch",
    },
    AccessCheck {
        group: "apps",
        resource: "deployments",
        verb: "patch",
        label: "deployments patch",
    },
    AccessCheck {
        group: "gateway.networking.k8s.io",
        resource: "gateways",
        verb: "create",
        label: "gateways create",
    },
    AccessCheck {
        group: "gateway.networking.k8s.io",
        resource: "httproutes",
        verb: "create",
        label: "httproutes create",
    },
    AccessCheck {
        group: "networking.istio.io",
        resource: "virtualservices",
        verb: "list",
        label: "virtualservices list",
    },
    AccessCheck {
        group: "maistra.io",
        resource: "servicemeshmemberrolls",
        verb: "patch",
        label: "servicemeshmemberrolls patch (OSSM)",
    },
    AccessCheck {
        group: "route.openshift.io",
        resource: "routes",
        verb: "list",
        label: "routes list (OpenShift ingress migration)",
    },
];

/// Returns human-readable labels for permissions the spoke credentials lack.
pub async fn rollout_access_gaps(client: &Client) -> Result<Vec<String>, RemoteClientError> {
    let mut denied = Vec::new();
    for check in ROLLOUT_CHECKS {
        if !subject_can(client, check.group, check.resource, check.verb, None).await? {
            denied.push(check.label.to_string());
        }
    }
    Ok(denied)
}

/// True when the remote identity can run hub-orchestrated migration rollouts.
pub async fn verify_rollout_access(client: &Client) -> Result<(), RemoteClientError> {
    let gaps = rollout_access_gaps(client).await?;
    if gaps.is_empty() {
        return Ok(());
    }
    Err(RemoteClientError::InvalidSecret(format!(
        "spoke RBAC insufficient for rollout (missing: {}). On the spoke run: \
         kubectl apply -f docs/lab/spoke-hub-remote-rbac.yaml",
        gaps.join(", ")
    )))
}

async fn subject_can(
    client: &Client,
    group: &str,
    resource: &str,
    verb: &str,
    namespace: Option<&str>,
) -> Result<bool, RemoteClientError> {
    let group_opt = if group.is_empty() {
        None
    } else {
        Some(group.to_string())
    };
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                namespace: namespace.map(str::to_string),
                verb: Some(verb.to_string()),
                group: group_opt,
                resource: Some(resource.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());
    let resp = api.create(&Default::default(), &review).await?;
    Ok(resp.status.is_some_and(|s| s.allowed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollout_checks_cover_enrollment_and_policy() {
        let labels: Vec<_> = ROLLOUT_CHECKS.iter().map(|c| c.label).collect();
        assert!(labels.iter().any(|l| l.contains("namespaces")));
        assert!(labels.iter().any(|l| l.contains("gateways")));
        assert!(labels.iter().any(|l| l.contains("httproutes")));
    }
}
