use std::path::PathBuf;

use ambientor_analyze::virtual_service_to_httproute;
use ambientor_core::inventory::AssessmentResult;
use ambientor_k8s::K8sClient;
use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
use ambientor_plan::{
    build_export_yaml, build_plan, migration_plan_cr, namespaces_from_findings, plan_to_rollout,
    translation_name_for_vs,
};
use ambientor_types::{
    MigrationPlan, PolicyTranslation, PolicyTranslationSpec, PolicyTranslationStatus,
};
use anyhow::Context;
use kube::Api;

pub async fn plan_create(
    kubeconfig: Option<&str>,
    namespace_filter: Option<String>,
    out_dir: Option<PathBuf>,
    json: bool,
) -> anyhow::Result<()> {
    let assess = super::assess_direct(kubeconfig).await?;
    let namespaces: Vec<String> = if let Some(ns) = namespace_filter {
        vec![ns]
    } else {
        namespaces_from_findings(&assess.findings)
    };

    let assessment = AssessmentResult {
        findings: assess.findings.clone(),
        scores: assess.scores.clone(),
        summary: assess.summary.clone(),
    };
    let spec = build_plan(&assessment, &namespaces);
    let rollout = plan_to_rollout(&spec);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "plan": spec,
                "rollout": rollout,
                "scores": assess.scores,
                "summary": assess.summary,
            }))?
        );
        return Ok(());
    }

    let k8s = open_k8s(kubeconfig).await?;
    let plan_ns = namespaces
        .first()
        .cloned()
        .unwrap_or_else(|| "default".into());
    let plan_name = "ambientor-local-plan";
    let translations = collect_translations_for_namespaces(&k8s, &namespaces).await?;
    let plan = migration_plan_cr(plan_name, &plan_ns, spec.clone());

    let bundle =
        build_export_yaml(&plan, &translations, &rollout).map_err(|e| anyhow::anyhow!(e))?;

    if let Some(dir) = out_dir {
        std::fs::create_dir_all(&dir)?;
        let bundle_path = dir.join("migration-bundle.yaml");
        std::fs::write(&bundle_path, &bundle)?;
        std::fs::write(
            dir.join("plan.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "plan": spec,
                "rollout": rollout,
                "scores": assess.scores,
                "summary": assess.summary,
            }))?,
        )?;
        println!("Wrote {}", bundle_path.display());
        println!("Wrote {}", dir.join("plan.json").display());
    } else {
        println!("{bundle}");
    }
    Ok(())
}

pub async fn plan_export(
    kubeconfig: Option<&str>,
    api_url: Option<&str>,
    namespace: String,
    name: String,
    out_file: Option<PathBuf>,
) -> anyhow::Result<()> {
    let yaml = if let Some(base) = api_url {
        export_via_api(base, &namespace, &name).await?
    } else {
        export_via_cluster(kubeconfig, &namespace, &name).await?
    };

    if let Some(path) = out_file {
        std::fs::write(&path, &yaml)?;
        println!("Wrote {}", path.display());
    } else {
        print!("{yaml}");
    }
    Ok(())
}

async fn export_via_api(base: &str, namespace: &str, name: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let url = format!("{base}/api/v1/plans/{namespace}/{name}/export");
    let resp = client.get(&url).send().await?.error_for_status()?;
    Ok(resp.text().await?)
}

async fn export_via_cluster(
    kubeconfig: Option<&str>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<String> {
    let k8s = open_k8s(kubeconfig).await?;
    let plan_api: Api<MigrationPlan> = Api::namespaced(k8s.client.clone(), namespace);
    let plan = plan_api.get(name).await?;
    let pt_api: Api<PolicyTranslation> = Api::namespaced(k8s.client.clone(), namespace);
    let pt_list = pt_api.list(&kube::api::ListParams::default()).await?;
    let rollout = plan_to_rollout(&plan.spec);
    build_export_yaml(&plan, &pt_list.items, &rollout).map_err(|e| anyhow::anyhow!(e))
}

async fn open_k8s(kubeconfig: Option<&str>) -> anyhow::Result<K8sClient> {
    match kubeconfig {
        Some(p) => K8sClient::from_kubeconfig(Some(p)).await,
        None => K8sClient::in_cluster()
            .await
            .or(K8sClient::from_kubeconfig(None).await),
    }
    .context("connect to Kubernetes cluster")
}

async fn collect_translations_for_namespaces(
    k8s: &K8sClient,
    namespaces: &[String],
) -> anyhow::Result<Vec<PolicyTranslation>> {
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let mut out = Vec::new();
    for ns in namespaces {
        let items = list_cr_in_namespace(&k8s.client, &vs_ar, ns).await?;
        for vs in items {
            let Some(vs_name) = vs.metadata.name.clone() else {
                continue;
            };
            match virtual_service_to_httproute(ns, &vs_name, &vs.data) {
                Ok(result) => {
                    out.push(PolicyTranslation {
                        spec: PolicyTranslationSpec {
                            source_kind: "VirtualService".into(),
                            source_name: vs_name.clone(),
                            target_kind: "HTTPRoute".into(),
                            namespace: ns.clone(),
                        },
                        status: Some(PolicyTranslationStatus {
                            phase: "Ready".into(),
                            suggested_manifest: Some(result.manifest),
                            warnings: result.warnings,
                        }),
                        metadata: kube::api::ObjectMeta {
                            name: Some(translation_name_for_vs(&vs_name)),
                            namespace: Some(ns.clone()),
                            ..Default::default()
                        },
                    });
                }
                Err(e) => {
                    out.push(PolicyTranslation {
                        spec: PolicyTranslationSpec {
                            source_kind: "VirtualService".into(),
                            source_name: vs_name.clone(),
                            target_kind: "HTTPRoute".into(),
                            namespace: ns.clone(),
                        },
                        status: Some(PolicyTranslationStatus {
                            phase: "Failed".into(),
                            suggested_manifest: None,
                            warnings: vec![e],
                        }),
                        metadata: kube::api::ObjectMeta {
                            name: Some(translation_name_for_vs(&vs_name)),
                            namespace: Some(ns.clone()),
                            ..Default::default()
                        },
                    });
                }
            }
        }
    }
    Ok(out)
}
