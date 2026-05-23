use ambientor_k8s::K8sClient;
use ambientor_types::Rollout;
use anyhow::Context;
use kube::Api;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RolloutDetail {
    name: String,
    namespace: String,
    phase: String,
    current_stage: i32,
    approved_stage: i32,
    awaiting_approval: bool,
}

pub async fn rollout_status(
    api_url: Option<&str>,
    kubeconfig: Option<&str>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    if let Some(url) = api_url {
        let detail: RolloutDetail = reqwest::get(format!(
            "{}/api/v1/rollouts/{}/{name}",
            url.trim_end_matches('/'),
            namespace
        ))
        .await?
        .error_for_status()?
        .json()
        .await?;
        print_detail(&detail);
        return Ok(());
    }
    let k8s = k8s_client(kubeconfig).await?;
    let api: Api<Rollout> = Api::namespaced(k8s.client, namespace);
    let r = api.get(name).await?;
    let status = r.status.as_ref().context("rollout has no status")?;
    println!(
        "rollout {namespace}/{name}: phase={} currentStage={} approvedStage={} stages={}",
        status.phase,
        status.current_stage,
        status.approved_stage,
        r.spec.stages.len()
    );
    if status.phase == "AwaitingApproval" {
        let stage = r
            .spec
            .stages
            .get(status.current_stage as usize)
            .map(|s| s.name.as_str())
            .unwrap_or("?");
        println!(
            "awaiting approval for stage {} ({stage})",
            status.current_stage
        );
    }
    Ok(())
}

pub async fn rollout_approve(
    api_url: Option<&str>,
    kubeconfig: Option<&str>,
    namespace: &str,
    name: &str,
    stage: Option<i32>,
) -> anyhow::Result<()> {
    if let Some(url) = api_url {
        let client = reqwest::Client::new();
        let body = serde_json::json!({ "stage": stage, "actor": "cli" });
        let resp: serde_json::Value = client
            .post(format!(
                "{}/api/v1/rollouts/{namespace}/{name}/approve",
                url.trim_end_matches('/')
            ))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    let k8s = k8s_client(kubeconfig).await?;
    let api: Api<Rollout> = Api::namespaced(k8s.client.clone(), namespace);
    let r = api.get(name).await?;
    let status = r.status.as_ref().context("rollout has no status")?;
    let stage_to_approve = stage.unwrap_or(status.current_stage);
    let phase = if status.phase == "RolledBack" {
        "Pending".to_string()
    } else {
        status.phase.clone()
    };
    let patch = serde_json::json!({
        "status": {
            "approvedStage": stage_to_approve,
            "phase": phase,
        }
    });
    api.patch_status(name, &Default::default(), &kube::api::Patch::Merge(&patch))
        .await?;
    println!(
        "approved stage {stage_to_approve} for rollout {namespace}/{name} (operator will continue)"
    );
    Ok(())
}

fn print_detail(d: &RolloutDetail) {
    println!(
        "rollout {}/{}: phase={} currentStage={} approvedStage={}",
        d.namespace, d.name, d.phase, d.current_stage, d.approved_stage
    );
    if d.awaiting_approval {
        println!("awaiting approval for stage {}", d.current_stage);
    }
}

async fn k8s_client(kubeconfig: Option<&str>) -> anyhow::Result<K8sClient> {
    match kubeconfig {
        Some(p) => K8sClient::from_kubeconfig(Some(p)).await,
        None => K8sClient::in_cluster()
            .await
            .or(K8sClient::from_kubeconfig(None).await),
    }
}
