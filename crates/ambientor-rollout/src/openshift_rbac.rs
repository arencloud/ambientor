//! OpenShift: hub reader needs namespace admin to create Routes with `spec.host` on apps domains.

use k8s_openapi::api::rbac::v1::RoleBinding;
use kube::api::{DeleteParams, PostParams};
use kube::{Api, Client};
use tracing::info;

use crate::engine::RolloutError;

pub const HUB_READER_SA_NAMESPACE: &str = "ambientor-system";
pub const HUB_READER_SA_NAME: &str = "ambientor-hub-reader";
pub const ROUTE_ADMIN_BINDING_NAME: &str = "ambientor-hub-reader-route-admin";
const MANAGED_LABEL: &str = "app.kubernetes.io/managed-by";
const MANAGED_VALUE: &str = "ambientor";

/// Grant namespace admin to the hub reader SA so OpenShift admits Routes with custom hosts.
pub async fn ensure_hub_route_admin(client: &Client, namespace: &str) -> Result<(), RolloutError> {
    let api: Api<RoleBinding> = Api::namespaced(client.clone(), namespace);
    if api.get(ROUTE_ADMIN_BINDING_NAME).await.is_ok() {
        return Ok(());
    }
    let binding = RoleBinding {
        metadata: kube::api::ObjectMeta {
            name: Some(ROUTE_ADMIN_BINDING_NAME.into()),
            labels: Some(
                [(MANAGED_LABEL.into(), MANAGED_VALUE.into())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        },
        role_ref: k8s_openapi::api::rbac::v1::RoleRef {
            api_group: "rbac.authorization.k8s.io".into(),
            kind: "ClusterRole".into(),
            name: "admin".into(),
        },
        subjects: Some(vec![k8s_openapi::api::rbac::v1::Subject {
            kind: "ServiceAccount".into(),
            name: HUB_READER_SA_NAME.into(),
            namespace: Some(HUB_READER_SA_NAMESPACE.into()),
            ..Default::default()
        }]),
    };
    api.create(&PostParams::default(), &binding).await?;
    info!(namespace = %namespace, "granted hub reader namespace admin for OpenShift Route host admission");
    Ok(())
}

/// Remove the temporary namespace admin binding after rollback.
pub async fn revoke_hub_route_admin(client: &Client, namespace: &str) -> Result<(), RolloutError> {
    let api: Api<RoleBinding> = Api::namespaced(client.clone(), namespace);
    match api.delete(ROUTE_ADMIN_BINDING_NAME, &DeleteParams::default()).await {
        Ok(_) => {
            info!(namespace = %namespace, "revoked hub reader route admin RoleBinding");
            Ok(())
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
        Err(e) => Err(RolloutError::Kube(e)),
    }
}
