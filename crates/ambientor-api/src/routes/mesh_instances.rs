use std::sync::Arc;

use ambientor_mesh::mesh_instances::discover_mesh_instances;
use ambientor_types::MeshInstance;
use axum::{Json, extract::State, http::StatusCode};

use crate::state::AppState;

use super::plans::{internal, k8s_client};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshInstanceListItem {
    pub revision: String,
    pub discovery_label: String,
    pub control_plane_namespace: String,
    pub version: Option<String>,
    pub ambient: bool,
    pub enrolled_namespace_count: usize,
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
            auto_select: false,
        }
    }
}

/// List Istio / OSSM control-plane instances (for rollout meshTarget selection).
pub async fn list_mesh_instances(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<MeshInstanceListItem>>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let instances = discover_mesh_instances(&k8s.client)
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
