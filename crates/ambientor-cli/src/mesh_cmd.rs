use ambientor_k8s::K8sClient;
use ambientor_mesh::{
    enroll_namespace_on_mesh,
    mesh_instances::{discover_mesh_instances, resolve_mesh_target},
};
use ambientor_types::MeshTarget;
use anyhow::Context;

pub async fn list_mesh_instances(kubeconfig: Option<&str>) -> anyhow::Result<()> {
    let k8s = k8s_client(kubeconfig).await?;
    let instances = discover_mesh_instances(&k8s.client).await?;
    println!("{}", serde_json::to_string_pretty(&instances)?);
    Ok(())
}

pub async fn enroll_namespace(
    kubeconfig: Option<&str>,
    namespace: &str,
    mesh_target: MeshTarget,
) -> anyhow::Result<()> {
    let k8s = k8s_client(kubeconfig).await?;
    let instances = discover_mesh_instances(&k8s.client).await?;
    let mesh =
        resolve_mesh_target(&instances, Some(&mesh_target)).map_err(|e| anyhow::anyhow!("{e}"))?;
    let actions = enroll_namespace_on_mesh(&k8s.client, namespace, &mesh).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "namespace": namespace,
            "mesh": mesh,
            "actions": actions,
        }))?
    );
    Ok(())
}

async fn k8s_client(kubeconfig: Option<&str>) -> anyhow::Result<K8sClient> {
    match kubeconfig {
        Some(p) => K8sClient::from_kubeconfig(Some(p)).await,
        None => K8sClient::in_cluster()
            .await
            .or(K8sClient::from_kubeconfig(None).await),
    }
    .context("connect to kubernetes")
}
