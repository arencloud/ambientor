use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_db::StoredAssessment;
use ambientor_k8s::{
    client_for_connection, connection_cluster_ref, verify_connectivity, K8sClient,
};
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_mesh::inventory::collect_inventory_full;
use ambientor_scan::default_registry;
use ambientor_types::{ClusterConnection, ClusterConnectionSpec, FindingSummary, SecretRef};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use kube::api::{DeleteParams, PostParams};
use kube::{Api, ResourceExt};
use serde::{Deserialize, Serialize};

use crate::routes::applications::persist_assessment_from_inventory;
use crate::routes::assess::{application_count_for_cluster, AssessRequest, AssessResponse};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionListItem {
    pub name: String,
    pub namespace: String,
    pub display_name: String,
    pub cluster_ref: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollout_access: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollout_access_message: Option<String>,
    pub hub: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_server: Option<String>,
    pub credentials_secret_ref: SecretRefResponse,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SecretRefResponse {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDetailResponse {
    #[serde(flatten)]
    pub item: ConnectionListItem,
    pub gitops_hint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConnectionRequest {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub display_name: String,
    #[serde(default)]
    pub api_server: Option<String>,
    pub credentials_secret_ref: SecretRefRequest,
    #[serde(default)]
    pub hub: bool,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SecretRefRequest {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionExportResponse {
    pub yaml: String,
    pub apply_command: String,
}

pub async fn list_connections(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<ConnectionListItem>>, (StatusCode, String)> {
    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::all(hub.client.clone());
    let list = api.list(&Default::default()).await.map_err(internal)?;
    let items = list
        .items
        .into_iter()
        .filter_map(connection_to_item)
        .collect();
    Ok(Json(items))
}

pub async fn get_connection(
    State(_state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<ConnectionDetailResponse>, (StatusCode, String)> {
    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::namespaced(hub.client.clone(), &namespace);
    let conn = api.get(&name).await.map_err(map_kube_err)?;
    let item = connection_to_item(conn.clone()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "invalid ClusterConnection object".into(),
    ))?;
    Ok(Json(ConnectionDetailResponse {
        gitops_hint: gitops_hint(&conn),
        item,
    }))
}

pub async fn create_connection(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<CreateConnectionRequest>,
) -> Result<Json<ConnectionDetailResponse>, (StatusCode, String)> {
    validate_connection_name(&body.name)?;
    let namespace = body
        .namespace
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ambientor-system".into());
    if body.name.is_empty() || body.display_name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name and displayName are required".into()));
    }
    if body.credentials_secret_ref.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "credentialsSecretRef.name is required".into(),
        ));
    }

    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::namespaced(hub.client.clone(), &namespace);
    let conn = ClusterConnection {
        metadata: kube::core::ObjectMeta {
            name: Some(body.name.clone()),
            namespace: Some(namespace.clone()),
            ..Default::default()
        },
        spec: ClusterConnectionSpec {
            display_name: body.display_name.trim().to_string(),
            api_server: body.api_server.filter(|s| !s.trim().is_empty()),
            credentials_secret_ref: SecretRef {
                name: body.credentials_secret_ref.name.trim().to_string(),
                namespace: body
                    .credentials_secret_ref
                    .namespace
                    .filter(|s| !s.is_empty()),
            },
            hub: body.hub,
        },
        status: None,
    };

    let created = api
        .create(&PostParams::default(), &conn)
        .await
        .map_err(map_kube_err)?;
    let item = connection_to_item(created.clone()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "created ClusterConnection missing metadata".into(),
    ))?;
    Ok(Json(ConnectionDetailResponse {
        gitops_hint: gitops_hint(&created),
        item,
    }))
}

pub async fn export_connection(
    State(_state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<ConnectionExportResponse>, (StatusCode, String)> {
    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::namespaced(hub.client.clone(), &namespace);
    let conn = api.get(&name).await.map_err(map_kube_err)?;
    let yaml = connection_export_yaml(&conn)?;
    let apply_command = format!("kubectl apply -f {name}.yaml  # namespace {namespace}");
    Ok(Json(ConnectionExportResponse { yaml, apply_command }))
}

pub async fn delete_connection(
    State(_state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::namespaced(hub.client.clone(), &namespace);
    let conn = api.get(&name).await.map_err(map_kube_err)?;
    if conn.spec.hub {
        return Err((
            StatusCode::BAD_REQUEST,
            "refusing to delete hub ClusterConnection; manage via GitOps on the hub".into(),
        ));
    }
    api.delete(&name, &DeleteParams::default())
        .await
        .map_err(map_kube_err)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn assess_connection(
    State(state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
    Json(body): Json<AssessRequest>,
) -> Result<Json<AssessResponse>, (StatusCode, String)> {
    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::namespaced(hub.client.clone(), &namespace);
    let conn = api.get(&name).await.map_err(map_kube_err)?;
    if conn.spec.hub {
        return Err((
            StatusCode::BAD_REQUEST,
            "cannot assess hub-local connection; use POST /api/v1/assess".into(),
        ));
    }

    let remote = client_for_connection(&hub.client, &conn)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    verify_connectivity(&remote.client).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("remote cluster unreachable: {e}"),
        )
    })?;

    let namespace_filter = body.namespace.as_deref();
    let platform = ambientor_k8s::detect_platform(&remote.client)
        .await
        .map_err(internal)?;
    let backend = backend_for_flavor(platform.mesh_flavor);
    let inventory = collect_inventory_full(&remote.client, platform.mesh_flavor, None)
        .await
        .map_err(internal)?;
    let mut ctx = inventory.ctx.clone();
    if let Ok(Some(ver)) = backend.detect_version(&remote.client).await {
        ctx.mesh_version = Some(ver);
    }

    let registry = default_registry();
    let findings = registry.evaluate_all(&ctx);
    let scores = compute_scores(&findings);
    let summary = FindingSummary::from_findings(&findings);
    let cluster_ref = connection_cluster_ref(&namespace, &name);

    state.sse.write().await.publish(
        "assessment",
        &serde_json::json!({
            "phase": "completed",
            "findingCount": findings.len(),
            "clusterRef": cluster_ref,
        }),
    );

    if let Some(repo) = state.scan_store() {
        let payload = StoredAssessment {
            findings: findings.clone(),
            scores: scores.clone(),
            summary: summary.clone(),
            source: Some(format!("connection:{namespace}/{name}")),
            assessment_name: None,
        };
        if let Err(e) = repo
            .record_completed(&cluster_ref, namespace_filter, &payload)
            .await
        {
            tracing::warn!(error = %e, cluster_ref = %cluster_ref, "failed to persist remote scan");
        }
    }

    if let Err(e) = persist_assessment_from_inventory(
        state.as_ref(),
        Some(&hub.client),
        &remote.client,
        &cluster_ref,
        &inventory,
        &findings,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            cluster_ref = %cluster_ref,
            "failed to persist remote application assessments"
        );
    }

    let application_count = application_count_for_cluster(state.as_ref(), &cluster_ref).await;

    Ok(Json(AssessResponse {
        findings,
        scores,
        summary,
        application_count,
        trigger: "direct".into(),
        assessment_name: None,
        assessment_namespace: None,
    }))
}

fn connection_to_item(conn: ClusterConnection) -> Option<ConnectionListItem> {
    let name = conn.metadata.name?;
    let namespace = conn.metadata.namespace.unwrap_or_else(|| "default".into());
    let cluster_ref = if conn.spec.hub {
        ambientor_db::cluster_ref_from_env()
    } else {
        connection_cluster_ref(&namespace, &name)
    };
    let status = conn.status.unwrap_or_default();
    let ready_message = status
        .conditions
        .iter()
        .find(|c| c.r#type == "Ready")
        .and_then(|c| c.message.clone());
    let rollout_condition = status
        .conditions
        .iter()
        .find(|c| c.r#type == "RolloutAccess");
    let rollout_access = rollout_condition.map(|c| c.status == "True");
    let rollout_access_message = rollout_condition.and_then(|c| c.message.clone());
    Some(ConnectionListItem {
        name,
        namespace,
        display_name: conn.spec.display_name,
        cluster_ref,
        phase: status.phase,
        last_sync_time: status.last_sync_time.map(|t| t.to_rfc3339()),
        ready_message,
        rollout_access,
        rollout_access_message,
        hub: conn.spec.hub,
        api_server: conn.spec.api_server,
        credentials_secret_ref: SecretRefResponse {
            name: conn.spec.credentials_secret_ref.name,
            namespace: conn.spec.credentials_secret_ref.namespace,
        },
    })
}

fn validate_connection_name(name: &str) -> Result<(), (StatusCode, String)> {
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name is required".into()));
    }
    if name.len() > 63 {
        return Err((StatusCode::BAD_REQUEST, "name too long".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
        || name.starts_with('-')
        || name.ends_with('-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "name must be a valid Kubernetes resource name".into(),
        ));
    }
    Ok(())
}

fn gitops_hint(conn: &ClusterConnection) -> String {
    let ns = conn.namespace().unwrap_or_else(|| "ambientor-system".into());
    let name = conn.name_any();
    format!(
        "Same object as `kubectl apply -f` / GitOps: ClusterConnection {ns}/{name}. Portal create/update writes this CR on the hub; CLI and operator reconcile from spec."
    )
}

fn connection_export_yaml(conn: &ClusterConnection) -> Result<String, (StatusCode, String)> {
    #[derive(Serialize)]
    struct ExportDoc<'a> {
        api_version: &'static str,
        kind: &'static str,
        metadata: ExportMeta<'a>,
        spec: ExportSpec<'a>,
    }
    #[derive(Serialize)]
    struct ExportMeta<'a> {
        name: &'a str,
        namespace: &'a str,
    }
    #[derive(Serialize)]
    struct ExportSpec<'a> {
        display_name: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        api_server: Option<&'a str>,
        credentials_secret_ref: &'a SecretRef,
        hub: bool,
    }

    let name = conn
        .metadata
        .name
        .as_deref()
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "missing name".into()))?;
    let namespace = conn
        .metadata
        .namespace
        .as_deref()
        .unwrap_or("ambientor-system");
    let doc = ExportDoc {
        api_version: "ambientor.io/v1alpha1",
        kind: "ClusterConnection",
        metadata: ExportMeta { name, namespace },
        spec: ExportSpec {
            display_name: &conn.spec.display_name,
            api_server: conn.spec.api_server.as_deref(),
            credentials_secret_ref: &conn.spec.credentials_secret_ref,
            hub: conn.spec.hub,
        },
    };
    serde_yaml::to_string(&doc).map_err(|e| internal(e))
}

async fn hub_client() -> Result<K8sClient, (StatusCode, String)> {
    K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(internal)
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn map_kube_err(e: kube::Error) -> (StatusCode, String) {
    match e {
        kube::Error::Api(err) if err.code == 404 => (StatusCode::NOT_FOUND, err.to_string()),
        kube::Error::Api(err) if err.code == 409 => {
            (StatusCode::CONFLICT, format!("ClusterConnection already exists: {err}"))
        }
        other => internal(other),
    }
}
