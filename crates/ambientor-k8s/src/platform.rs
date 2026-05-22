use ambientor_types::MeshFlavor;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};

#[derive(Clone, Debug, Default)]
pub struct PlatformInfo {
    pub is_openshift: bool,
    pub mesh_flavor: MeshFlavor,
    pub version: Option<String>,
}

pub async fn detect_platform(client: &Client) -> anyhow::Result<PlatformInfo> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&Default::default()).await?;

    let has_istio = crds
        .items
        .iter()
        .any(|c| c.metadata.name.as_deref() == Some("istios.sailoperator.io"));
    let has_ossm_member = crds.items.iter().any(|c| {
        c.metadata
            .name
            .as_deref()
            .is_some_and(|n| n.contains("servicemeshmemberrolls"))
    });
    let has_istio_op = crds.items.iter().any(|c| {
        c.metadata
            .name
            .as_deref()
            .is_some_and(|n| n.contains("istiooperators"))
    });

    let is_openshift = crds.items.iter().any(|c| {
        c.metadata
            .name
            .as_deref()
            .is_some_and(|n| n.contains("routes.openshift.io"))
    });

    let mesh_flavor = if has_istio || has_ossm_member {
        MeshFlavor::OSSM3
    } else if has_istio_op {
        MeshFlavor::UpstreamIstio
    } else {
        MeshFlavor::GenericKubernetes
    };

    Ok(PlatformInfo {
        is_openshift,
        mesh_flavor,
        version: None,
    })
}
