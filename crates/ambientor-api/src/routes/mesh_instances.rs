use std::sync::Arc;

use ambientor_k8s::client_for_cluster_ref;
use ambientor_mesh::enroll_namespace_on_mesh;
use ambientor_mesh::mesh_instances::{discover_mesh_instances, resolve_mesh_target};
use ambientor_types::{MeshEnrollment, MeshInstance};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};

use crate::state::AppState;

use super::plans::{internal, k8s_client};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshInstancesQuery {
    pub cluster_ref: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshInstanceListItem {
    pub revision: String,
    pub discovery_label: String,
    pub control_plane_namespace: String,
    pub version: Option<String>,
    pub ambient: bool,
    pub enrolled_namespace_count: usize,
    pub enrollment: MeshEnrollment,
    /// True when this is the only ambient instance (safe to omit rollout.spec.meshTarget).
    pub auto_select: bool,
}

impl From<MeshInstance> for MeshInstanceListItem {
    fn from(i: MeshInstance) -> Self {
        Self {
            revision: i.revision,
            discovery_label: i.discovery_label,
            control_plane_namespace: i.control_plane_namespace,
            version: i.version,
            ambient: i.ambient,
            enrolled_namespace_count: i.enrolled_namespace_count,
            enrollment: i.enrollment,
            auto_select: false,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollNamespaceRequest {
    pub namespace: String,
    #[serde(default)]
    pub mesh_target: Option<ambientor_types::MeshTarget>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollNamespaceResponse {
    pub namespace: String,
    pub mesh: MeshInstanceListItem,
    pub actions: Vec<String>,
}

/// List Istio / OSSM control-plane instances (for rollout meshTarget selection).
pub async fn list_mesh_instances(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<MeshInstancesQuery>,
) -> Result<Json<Vec<MeshInstanceListItem>>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let exec_client = client_for_cluster_ref(
        &k8s.client,
        query.cluster_ref.as_deref().filter(|s| !s.is_empty()),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("target cluster client: {e}"),
        )
    })?;
    let instances = discover_mesh_instances(&exec_client)
        .await
        .map_err(internal)?;
    let ambient_count = instances.iter().filter(|i| i.ambient).count();
    let mut items: Vec<MeshInstanceListItem> = instances.into_iter().map(Into::into).collect();
    if ambient_count == 1 {
        for item in &mut items {
            item.auto_select = item.ambient;
        }
    }
    Ok(Json(items))
}

/// Enroll a namespace on a mesh instance (same logic as rollout `EnrollNamespace` stage).
pub async fn enroll_namespace(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<MeshInstancesQuery>,
    Json(body): Json<EnrollNamespaceRequest>,
) -> Result<Json<EnrollNamespaceResponse>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let exec_client = client_for_cluster_ref(
        &k8s.client,
        query.cluster_ref.as_deref().filter(|s| !s.is_empty()),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("target cluster client: {e}"),
        )
    })?;
    let instances = discover_mesh_instances(&exec_client)
        .await
        .map_err(internal)?;
    let mesh = resolve_mesh_target(&instances, body.mesh_target.as_ref())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let actions = enroll_namespace_on_mesh(&exec_client, &body.namespace, &mesh)
        .await
        .map_err(internal)?;
    Ok(Json(EnrollNamespaceResponse {
        namespace: body.namespace,
        mesh: mesh.into(),
        actions,
    }))
}
