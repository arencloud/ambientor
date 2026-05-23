use ambientor_analyze::virtual_service_to_httproute;
use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
use ambientor_plan::translation_name_for_vs;
use ambientor_types::{PolicyTranslation, PolicyTranslationSpec};
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use serde_yaml;
use tracing::{info, warn};

use crate::apply::apply_namespaced_manifest;
use crate::engine::{FIELD_MANAGER, RolloutError};

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

async fn upsert_translation_cr(
    client: &Client,
    namespace: &str,
    vs_name: &str,
    manifest: &str,
    warnings: &[String],
) -> Result<(), RolloutError> {
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
    api.patch(&cr_name, &pp, &Patch::Apply(&cr))
        .await
        .map_err(RolloutError::Kube)?;

    let status = serde_json::json!({
        "status": {
            "phase": "Ready",
            "suggestedManifest": manifest,
            "warnings": warnings,
        }
    });
    api.patch_status(&cr_name, &Default::default(), &Patch::Merge(&status))
        .await
        .map_err(RolloutError::Kube)?;
    Ok(())
}
