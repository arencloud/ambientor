use std::sync::Arc;

use ambientor_analyze::virtual_service_to_httproute;
use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
use ambientor_types::{PolicyTranslation, PolicyTranslationSpec};
use futures::StreamExt;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};
use tracing::info;

use super::inventory::FIELD_MANAGER;
use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub fn translation_name_for_vs(vs_name: &str) -> String {
    format!("{vs_name}-translation")
}

pub async fn run(client: Client) {
    Controller::new(
        Api::<PolicyTranslation>::all(client.clone()),
        Config::default(),
    )
    .shutdown_on_signal()
    .run(reconcile, error_policy, Arc::new(client))
    .for_each(|res| async move {
        if let Err(e) = res {
            tracing::error!(error = %e, "policytranslation controller error");
        }
    })
    .await;
}

async fn reconcile(obj: Arc<PolicyTranslation>, client: Arc<Client>) -> ReconcileResult {
    let phase = obj.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
    if phase == "Ready" {
        return Ok(Action::await_change());
    }
    reconcile_inner(&client, &obj)
        .await
        .map_err(ReconcileError::Other)?;
    Ok(Action::await_change())
}

async fn reconcile_inner(client: &Client, obj: &PolicyTranslation) -> anyhow::Result<()> {
    let cr_ns = obj
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let name = obj
        .metadata
        .name
        .clone()
        .ok_or_else(|| anyhow::anyhow!("PolicyTranslation missing metadata.name"))?;

    if obj.spec.source_kind != "VirtualService" || obj.spec.target_kind != "HTTPRoute" {
        patch_status(
            client,
            &cr_ns,
            &name,
            "Failed",
            None,
            vec!["Only VirtualService → HTTPRoute is supported".into()],
        )
        .await?;
        return Ok(());
    }

    let vs_ns = if obj.spec.namespace.is_empty() {
        cr_ns.clone()
    } else {
        obj.spec.namespace.clone()
    };

    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let vs_api = Api::<kube::api::DynamicObject>::namespaced_with(client.clone(), &vs_ns, &vs_ar);
    let vs = match vs_api.get(&obj.spec.source_name).await {
        Ok(v) => v,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            patch_status(
                client,
                &cr_ns,
                &name,
                "Failed",
                None,
                vec![format!(
                    "VirtualService {}/{} not found",
                    vs_ns, obj.spec.source_name
                )],
            )
            .await?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    match virtual_service_to_httproute(&vs_ns, &obj.spec.source_name, &vs.data) {
        Ok(result) => {
            patch_status(
                client,
                &cr_ns,
                &name,
                "Ready",
                Some(&result.manifest),
                result.warnings,
            )
            .await?;
            info!(
                translation = %name,
                vs = %obj.spec.source_name,
                namespace = %vs_ns,
                "policy translation ready"
            );
        }
        Err(e) => {
            patch_status(client, &cr_ns, &name, "Failed", None, vec![e]).await?;
        }
    }
    Ok(())
}

async fn patch_status(
    client: &Client,
    ns: &str,
    name: &str,
    phase: &str,
    suggested_manifest: Option<&str>,
    warnings: Vec<String>,
) -> anyhow::Result<()> {
    let api: Api<PolicyTranslation> = Api::namespaced(client.clone(), ns);
    let status = serde_json::json!({
        "status": {
            "phase": phase,
            "suggestedManifest": suggested_manifest,
            "warnings": warnings,
        }
    });
    api.patch_status(name, &Default::default(), &Patch::Merge(&status))
        .await?;
    Ok(())
}

/// Ensure a PolicyTranslation CR exists for a VirtualService (controller fills status).
pub async fn ensure_translation_for_vs(
    client: &Client,
    namespace: &str,
    vs_name: &str,
) -> anyhow::Result<()> {
    let cr_name = translation_name_for_vs(vs_name);
    let cr = PolicyTranslation::new(
        &cr_name,
        PolicyTranslationSpec {
            source_kind: "VirtualService".into(),
            source_name: vs_name.to_string(),
            target_kind: "HTTPRoute".into(),
            namespace: namespace.to_string(),
        },
    );
    let api: Api<PolicyTranslation> = Api::namespaced(client.clone(), namespace);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(&cr_name, &pp, &Patch::Apply(&cr)).await?;
    Ok(())
}

/// Create translation CRs for all VirtualServices in a namespace.
pub async fn ensure_translations_in_namespace(
    client: &Client,
    namespace: &str,
) -> anyhow::Result<()> {
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let items = list_cr_in_namespace(client, &vs_ar, namespace).await?;
    for vs in items {
        if let Some(name) = vs.metadata.name
            && let Err(e) = ensure_translation_for_vs(client, namespace, &name).await
        {
            tracing::warn!(
                error = %e,
                namespace = %namespace,
                vs = %name,
                "failed to ensure PolicyTranslation"
            );
        }
    }
    Ok(())
}
