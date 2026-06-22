use ambientor_analyze::virtual_service_to_httproute;
use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
use ambientor_plan::translation_name_for_vs;
use ambientor_types::{PolicyTranslation, PolicyTranslationSpec};
use kube::{
    Api, Client,
    api::{DynamicObject, Patch, PatchParams},
};
use serde_json::json;
use serde_yaml;
use tracing::{info, warn};

use kube::api::DeleteParams;

use crate::apply::apply_namespaced_manifest;
use crate::engine::{FIELD_MANAGER, RolloutError};

const TRANSLATED_FROM_LABEL: &str = "ambientor.io/translated-from";
const TRANSLATED_FROM_VALUE: &str = "VirtualService";
const ORIGINAL_VS_GATEWAYS_ANNOTATION: &str = "ambientor.io/original-vs-gateways";

/// Translate VirtualServices to HTTPRoutes and apply them; upsert PolicyTranslation CRs.
pub async fn translate_policies_in_namespace(
    client: &Client,
    namespace: &str,
) -> Result<usize, RolloutError> {
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let vs_list = list_cr_in_namespace(client, &vs_ar, namespace)
        .await
        .map_err(|e| {
            RolloutError::ExecutionFailed(format!("list VirtualServices in {namespace}: {e}"))
        })?;
    if vs_list.is_empty() {
        return Ok(0);
    }

    let mut applied = 0usize;
    let mut failures = Vec::new();
    for vs in vs_list {
        let Some(vs_name) = vs.metadata.name.clone() else {
            continue;
        };
        match virtual_service_to_httproute(namespace, &vs_name, &vs.data) {
            Ok(result) => {
                let manifest: serde_json::Value = serde_yaml::from_str(&result.manifest)
                    .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
                apply_namespaced_manifest(client, namespace, &manifest).await?;
                retire_virtual_service_ingress(client, namespace, &vs).await?;
                upsert_translation_cr(
                    client,
                    namespace,
                    &vs_name,
                    &result.manifest,
                    &result.warnings,
                )
                .await?;
                applied += 1;
                info!(
                    namespace = %namespace,
                    vs = %vs_name,
                    "applied HTTPRoute policy translation"
                );
            }
            Err(e) => {
                warn!(
                    namespace = %namespace,
                    vs = %vs_name,
                    error = %e,
                    "skipped VirtualService translation"
                );
                failures.push(format!("{vs_name}: {e}"));
            }
        }
    }

    if applied == 0 {
        return Err(RolloutError::ExecutionFailed(format!(
            "no VirtualService translations applied in {namespace}: {}",
            failures.join("; ")
        )));
    }
    Ok(applied)
}

/// Delete HTTPRoutes and PolicyTranslations applied by Ambientor rollout.
pub async fn revert_translations_in_namespace(
    client: &Client,
    namespace: &str,
) -> Result<usize, RolloutError> {
    let hr_ar = api_resource("gateway.networking.k8s.io", "v1", "HTTPRoute", "httproutes");
    let routes = list_cr_in_namespace(client, &hr_ar, namespace)
        .await
        .map_err(|e| {
            RolloutError::ExecutionFailed(format!("list HTTPRoutes in {namespace}: {e}"))
        })?;
    let mut removed = 0usize;
    let hr_api =
        kube::Api::<kube::api::DynamicObject>::namespaced_with(client.clone(), namespace, &hr_ar);
    for route in routes {
        let managed = route
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get(TRANSLATED_FROM_LABEL))
            .map(String::as_str)
            == Some(TRANSLATED_FROM_VALUE);
        if !managed {
            continue;
        }
        let Some(name) = route.metadata.name else {
            continue;
        };
        if let Some(vs_name) = route
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("ambientor.io/source-name"))
        {
            restore_virtual_service_ingress(client, namespace, vs_name).await?;
            delete_translation_cr(client, namespace, vs_name).await?;
        }
        match hr_api.delete(&name, &DeleteParams::default()).await {
            Ok(_) => {
                removed += 1;
                info!(namespace = %namespace, route = %name, "removed translated HTTPRoute");
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {}
            Err(e) => return Err(RolloutError::Kube(e)),
        }
    }
    Ok(removed)
}

async fn delete_translation_cr(
    client: &Client,
    namespace: &str,
    vs_name: &str,
) -> Result<(), RolloutError> {
    let cr_name = translation_name_for_vs(vs_name);
    let api: Api<PolicyTranslation> = Api::namespaced(client.clone(), namespace);
    match api.delete(&cr_name, &DeleteParams::default()).await {
        Ok(_) => info!(namespace = %namespace, cr = %cr_name, "removed PolicyTranslation"),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(RolloutError::Kube(e)),
    }
    Ok(())
}

async fn upsert_translation_cr(
    client: &Client,
    namespace: &str,
    vs_name: &str,
    manifest: &str,
    warnings: &[String],
) -> Result<(), RolloutError> {
    let cr_name = translation_name_for_vs(vs_name);
    let api: Api<PolicyTranslation> = Api::namespaced(client.clone(), namespace);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    let cr = PolicyTranslation::new(
        &cr_name,
        PolicyTranslationSpec {
            source_kind: "VirtualService".into(),
            source_name: vs_name.to_string(),
            target_kind: "HTTPRoute".into(),
            namespace: namespace.to_string(),
        },
    );
    if let Err(e) = api.patch(&cr_name, &pp, &Patch::Apply(&cr)).await {
        if is_policy_translation_crd_missing(&e) {
            info!(
                namespace = %namespace,
                vs = %vs_name,
                "PolicyTranslation CRD not installed on spoke; HTTPRoute translation applied without CR"
            );
            return Ok(());
        }
        return Err(RolloutError::Kube(e));
    }

    let status = json!({
        "status": {
            "phase": "Ready",
            "suggestedManifest": manifest,
            "warnings": warnings,
        }
    });
    if let Err(e) = api
        .patch_status(&cr_name, &Default::default(), &Patch::Merge(&status))
        .await
    {
        if is_policy_translation_crd_missing(&e) {
            return Ok(());
        }
        return Err(RolloutError::Kube(e));
    }
    Ok(())
}

fn is_policy_translation_crd_missing(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(e) if e.code == 404 => true,
        _ => false,
    }
}

/// Detach the VirtualService from ingress gateways after HTTPRoute translation.
async fn retire_virtual_service_ingress(
    client: &Client,
    namespace: &str,
    vs: &DynamicObject,
) -> Result<(), RolloutError> {
    let Some(vs_name) = vs.metadata.name.clone() else {
        return Ok(());
    };
    let gateways = vs
        .data
        .pointer("/spec/gateways")
        .cloned()
        .unwrap_or(json!([]));
    if gateways.as_array().is_some_and(|a| a.is_empty()) {
        return Ok(());
    }
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &vs_ar);
    let patch = json!({
        "metadata": {
            "annotations": {
                ORIGINAL_VS_GATEWAYS_ANNOTATION: gateways.to_string()
            }
        },
        "spec": {
            "gateways": null
        }
    });
    api.patch(&vs_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(
        namespace = %namespace,
        vs = %vs_name,
        "detached VirtualService from ingress gateways after HTTPRoute translation"
    );
    Ok(())
}

async fn restore_virtual_service_ingress(
    client: &Client,
    namespace: &str,
    vs_name: &str,
) -> Result<(), RolloutError> {
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &vs_ar);
    let vs = match api.get(vs_name).await {
        Ok(v) => v,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(()),
        Err(e) => return Err(RolloutError::Kube(e)),
    };
    let gateways = vs
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(ORIGINAL_VS_GATEWAYS_ANNOTATION))
        .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok());
    let Some(gateways) = gateways else {
        return Ok(());
    };
    let patch = json!({
        "metadata": {
            "annotations": { ORIGINAL_VS_GATEWAYS_ANNOTATION: null }
        },
        "spec": { "gateways": gateways }
    });
    api.patch(vs_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(
        namespace = %namespace,
        vs = %vs_name,
        "restored VirtualService ingress gateway binding"
    );
    Ok(())
}
