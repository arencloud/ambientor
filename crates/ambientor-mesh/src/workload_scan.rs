use ambientor_core::rules::WorkloadContext;
use k8s_openapi::api::core::v1::Pod;

/// Build workload scan results from live pods (sidecar injection namespaces only).
pub fn scan_workloads(pods: &[Pod], injected_namespaces: &[String]) -> Vec<WorkloadContext> {
    let injected: std::collections::HashSet<_> = injected_namespaces.iter().collect();
    let mut out = Vec::new();
    for pod in pods {
        let Some(ns) = pod.metadata.namespace.as_ref() else {
            continue;
        };
        if !injected.contains(ns) {
            continue;
        }
        let name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };
        let has_sidecar = pod_has_istio_sidecar(pod);
        let (uses_localhost, hits) = pod_localhost_proxy_usage(pod);
        let hold_until = pod_hold_until_proxy(pod);
        if !has_sidecar && !uses_localhost && !hold_until {
            continue;
        }
        out.push(WorkloadContext {
            namespace: ns.clone(),
            name,
            has_istio_sidecar: has_sidecar,
            uses_localhost_proxy: uses_localhost,
            localhost_proxy_hits: hits,
            hold_until_proxy: hold_until,
        });
    }
    out
}

pub fn pod_has_istio_sidecar(pod: &Pod) -> bool {
    pod.spec
        .as_ref()
        .is_some_and(|spec| spec.containers.iter().any(|c| c.name == "istio-proxy"))
        || pod
            .metadata
            .annotations
            .as_ref()
            .is_some_and(|a| a.contains_key("sidecar.istio.io/status"))
}

pub fn pod_hold_until_proxy(pod: &Pod) -> bool {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("proxy.istio.io/config"))
        .is_some_and(|c| {
            c.contains("holdApplicationUntilProxyStarts")
                || c.contains("\"holdApplicationUntilProxyStarts\": true")
                || c.contains("holdApplicationUntilProxyStarts: true")
        })
}

pub fn pod_localhost_proxy_usage(pod: &Pod) -> (bool, Vec<String>) {
    let mut hits = Vec::new();
    let Some(spec) = pod.spec.as_ref() else {
        return (false, hits);
    };
    for container in &spec.containers {
        if container.name == "istio-proxy" {
            continue;
        }
        if let Some(env) = &container.env {
            for e in env {
                let key = e.name.as_str();
                if let Some(val) = &e.value
                    && value_uses_localhost_proxy(val)
                {
                    hits.push(format!("{}: env {key}={val}", container.name));
                }
            }
        }
        for part in container
            .args
            .iter()
            .flatten()
            .chain(container.command.iter().flatten())
        {
            if value_uses_localhost_proxy(part) {
                hits.push(format!("{}: arg/cmd {part}", container.name));
            }
        }
    }
    (!hits.is_empty(), hits)
}

pub fn value_uses_localhost_proxy(value: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "127.0.0.1:15000",
        "127.0.0.1:15001",
        "localhost:15000",
        "localhost:15001",
    ];
    NEEDLES.iter().any(|n| value.contains(n))
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{Container, EnvVar, Pod, PodSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

    fn pod_with_env(ns: &str, name: &str, env: Vec<EnvVar>) -> Pod {
        Pod {
            metadata: ObjectMeta {
                namespace: Some(ns.into()),
                name: Some(name.into()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "app".into(),
                    env: Some(env),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn detects_localhost_admin_port_in_env() {
        let pod = pod_with_env(
            "bookinfo",
            "reviews",
            vec![EnvVar {
                name: "METRICS".into(),
                value: Some("http://127.0.0.1:15000/stats".into()),
                ..Default::default()
            }],
        );
        let (uses, hits) = pod_localhost_proxy_usage(&pod);
        assert!(uses);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn detects_hold_until_proxy_annotation() {
        let mut pod = pod_with_env("ns", "p", vec![]);
        pod.metadata.annotations = Some(std::collections::BTreeMap::from([(
            "proxy.istio.io/config".into(),
            "{ \"holdApplicationUntilProxyStarts\": true }".into(),
        )]));
        assert!(pod_hold_until_proxy(&pod));
    }
}
