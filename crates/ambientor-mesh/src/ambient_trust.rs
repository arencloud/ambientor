//! Ensure istiod trusts the ztunnel service account for ambient workload identity (HBONE certs).

use ambientor_types::MeshInstance;
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{DynamicObject, Patch, PatchParams};
use kube::{Api, Client};
use tracing::info;

use crate::dynamic::{api_resource, list_cluster_cr};

const ZTUNNEL_SA_NAME: &str = "ztunnel";
const CA_TRUSTED_ENV: &str = "CA_TRUSTED_NODE_ACCOUNTS";

/// Resolve the `namespace/serviceaccount` identity ztunnel uses (from Sail `ZTunnel` CR or pods).
pub async fn ztunnel_trusted_account(client: &Client) -> Option<String> {
    if let Some(account) = ztunnel_account_from_cr(client).await {
        return Some(account);
    }
    ztunnel_account_from_pods(client).await
}

/// Read `CA_TRUSTED_NODE_ACCOUNTS` from the istiod Deployment for this mesh revision.
pub async fn istiod_ca_trusted_accounts(
    client: &Client,
    control_plane_namespace: &str,
    revision: &str,
) -> Option<String> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), control_plane_namespace);
    let candidates = [
        format!("istiod-{revision}"),
        "istiod".to_string(),
    ];
    for name in candidates {
        let Ok(dep) = api.get(&name).await else {
            continue;
        };
        if let Some(value) = deployment_env_value(&dep, CA_TRUSTED_ENV) {
            return Some(value);
        }
    }
    None
}

/// Patch the Sail `Istio` CR when istiod does not trust the live ztunnel service account.
///
/// Without this, ambient HBONE returns 503 (`upstream connect error`) because ztunnel cannot
/// impersonate workload service accounts for certificate signing.
pub async fn ensure_istiod_trusts_ztunnel(
    client: &Client,
    mesh: &MeshInstance,
) -> anyhow::Result<String> {
    if !mesh.ambient {
        return Ok("mesh is not ambient; ztunnel trust check skipped".into());
    }

    let expected = match ztunnel_trusted_account(client).await {
        Some(v) => v,
        None => {
            return Ok("no ztunnel found; ztunnel trust check skipped".into());
        }
    };

    let current = istiod_ca_trusted_accounts(
        client,
        &mesh.control_plane_namespace,
        &mesh.enrollment.revision,
    )
    .await;

    if current.as_deref() == Some(expected.as_str()) {
        return Ok(format!("istiod already trusts ztunnel ({expected})"));
    }

    let istio_cr = find_istio_cr_for_mesh(client, mesh).await?;
    let Some(istio) = istio_cr else {
        let cur = current.unwrap_or_else(|| "<unset>".into());
        anyhow::bail!(
            "istiod CA_TRUSTED_NODE_ACCOUNTS is '{cur}' but ztunnel runs as '{expected}'; \
             patch the Sail Istio CR (spec.values.pilot.env.{CA_TRUSTED_ENV}={expected}) \
             or grant ambientor patch on sailoperator.io/istios"
        );
    };

    let name = istio
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "unknown".into());
    let patch = serde_json::json!({
        "spec": {
            "values": {
                "pilot": {
                    "env": {
                        CA_TRUSTED_ENV: expected
                    }
                }
            }
        }
    });
    let ar = api_resource("sailoperator.io", "v1", "Istio", "istios");
    let api = Api::<DynamicObject>::all_with(client.clone(), &ar);
    api.patch(
        &name,
        &PatchParams::apply("ambientor.io").force(),
        &Patch::Merge(&patch),
    )
    .await?;

    let cur = current.unwrap_or_else(|| "<unset>".into());
    info!(
        istio_cr = %name,
        from = %cur,
        to = %expected,
        "patched istiod CA_TRUSTED_NODE_ACCOUNTS for ambient ztunnel identity"
    );
    Ok(format!(
        "patched Istio/{name}: {CA_TRUSTED_ENV} {cur} -> {expected}"
    ))
}

async fn ztunnel_account_from_cr(client: &Client) -> Option<String> {
    let ar = api_resource("sailoperator.io", "v1", "ZTunnel", "ztunnels");
    let items = list_cluster_cr(client, &ar).await.ok()?;
    let ztunnel = items.into_iter().next()?;
    let ns = ztunnel
        .data
        .get("spec")
        .and_then(|s| s.get("namespace"))
        .and_then(|v| v.as_str())?;
    Some(format!("{ns}/{ZTUNNEL_SA_NAME}"))
}

async fn ztunnel_account_from_pods(client: &Client) -> Option<String> {
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::ListParams;

    let api: Api<Pod> = Api::all(client.clone());
    let lp = ListParams::default().labels("app=ztunnel");
    let pods = api.list(&lp).await.ok()?;
    let pod = pods.items.first()?;
    let ns = pod.metadata.namespace.as_deref()?;
    let sa = pod
        .spec
        .as_ref()
        .map(|s| s.service_account_name.as_deref().unwrap_or(ZTUNNEL_SA_NAME))
        .unwrap_or(ZTUNNEL_SA_NAME);
    Some(format!("{ns}/{sa}"))
}

async fn find_istio_cr_for_mesh(
    client: &Client,
    mesh: &MeshInstance,
) -> anyhow::Result<Option<kube::api::DynamicObject>> {
    let ar = api_resource("sailoperator.io", "v1", "Istio", "istios");
    let items = list_cluster_cr(client, &ar).await?;
    Ok(items.into_iter().find(|istio| istio_cr_matches_mesh(istio, mesh)))
}

fn istio_cr_matches_mesh(istio: &kube::api::DynamicObject, mesh: &MeshInstance) -> bool {
    let data = &istio.data;
    if data
        .pointer("/status/activeRevisionName")
        .and_then(|v| v.as_str())
        == Some(mesh.enrollment.revision.as_str())
    {
        return true;
    }
    if data
        .pointer("/spec/namespace")
        .and_then(|v| v.as_str())
        == Some(mesh.control_plane_namespace.as_str())
    {
        return true;
    }
    false
}

fn deployment_env_value(dep: &Deployment, name: &str) -> Option<String> {
    dep.spec
        .as_ref()?
        .template
        .spec
        .as_ref()?
        .containers
        .iter()
        .find_map(|c| {
            c.env.as_ref().and_then(|envs| {
                envs.iter().find_map(|e| {
                    if e.name == name {
                        e.value.clone()
                    } else {
                        None
                    }
                })
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamic::api_resource;
    use kube::api::DynamicObject;
    use serde_json::json;

    fn mesh() -> MeshInstance {
        use ambientor_types::{MeshEnrollment, MeshEnrollmentMode, MeshInstance};
        let enrollment = MeshEnrollment {
            mode: MeshEnrollmentMode::RevisionAndDiscovery,
            revision: "ambient-v1-28-6".into(),
            istio_revision: Some("ambient-v1-28-6".into()),
            revision_tag: None,
            discovery_label_key: Some("istio-discovery".into()),
            discovery_label_value: Some("mesh-ambient".into()),
            member_roll_namespace: None,
            from_istiod_config: false,
        };
        MeshInstance {
            revision: enrollment.revision.clone(),
            discovery_label: "mesh-ambient".into(),
            control_plane_namespace: "ambient-istio-system".into(),
            version: Some("1.28.6".into()),
            ambient: true,
            enrolled_namespace_count: 1,
            enrollment,
        }
    }

    #[test]
    fn matches_istio_cr_by_active_revision() {
        let ar = api_resource("sailoperator.io", "v1", "Istio", "istios");
        let mut istio = DynamicObject::new("ambient", &ar);
        istio.data = json!({
            "status": { "activeRevisionName": "ambient-v1-28-6" }
        });
        assert!(istio_cr_matches_mesh(&istio, &mesh()));
    }

    #[test]
    fn reads_env_from_deployment() {
        let dep: Deployment = serde_json::from_value(json!({
            "spec": {
                "template": {
                    "spec": {
                        "containers": [{
                            "name": "discovery",
                            "env": [
                                { "name": "CA_TRUSTED_NODE_ACCOUNTS", "value": "ztunnel/ztunnel" }
                            ]
                        }]
                    }
                }
            }
        }))
        .unwrap();
        assert_eq!(
            deployment_env_value(&dep, CA_TRUSTED_ENV).as_deref(),
            Some("ztunnel/ztunnel")
        );
    }
}
