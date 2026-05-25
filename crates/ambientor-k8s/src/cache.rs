//! In-memory reflector caches for cluster-scoped core resources.

use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::{
    Api, Client,
    runtime::{
        reflector::{self, store, Store},
        watcher::{self, Config},
    },
};
use tracing::info;

/// Shared pod and namespace caches fed by kube-runtime reflectors.
#[derive(Clone)]
pub struct ClusterResourceCache {
    pods: Store<Pod>,
    namespaces: Store<Namespace>,
}

impl ClusterResourceCache {
    /// Start reflectors and return a handle to the local stores.
    pub fn spawn(client: Client) -> Self {
        let (pods, pod_writer) = store::store();
        let (namespaces, ns_writer) = store::store();

        let pod_api = Api::<Pod>::all(client.clone());
        let ns_api = Api::<Namespace>::all(client.clone());
        let pod_stream = reflector::reflector(pod_writer, watcher::watcher(pod_api, Config::default()));
        let ns_stream =
            reflector::reflector(ns_writer, watcher::watcher(ns_api, Config::default()));

        tokio::spawn(async move {
            info!("cluster resource cache: starting pod and namespace reflectors");
            let mut pod_stream = std::pin::pin!(pod_stream);
            let mut ns_stream = std::pin::pin!(ns_stream);
            tokio::join!(
                drain_reflector(&mut pod_stream),
                drain_reflector(&mut ns_stream),
            );
        });

        Self { pods, namespaces }
    }

    pub fn pod_count(&self) -> usize {
        self.pods.state().len()
    }

    pub fn namespace_count(&self) -> usize {
        self.namespaces.state().len()
    }

    /// True when at least one pod or namespace object is present in cache.
    pub fn is_populated(&self) -> bool {
        self.pod_count() > 0 || self.namespace_count() > 0
    }

    pub fn pod_snapshot(&self) -> Vec<Pod> {
        self.pods
            .state()
            .into_iter()
            .map(Arc::unwrap_or_clone)
            .collect()
    }

    pub fn namespace_snapshot(&self) -> Vec<Namespace> {
        self.namespaces
            .state()
            .into_iter()
            .map(Arc::unwrap_or_clone)
            .collect()
    }
}

async fn drain_reflector<S>(stream: &mut S)
where
    S: futures::Stream + Unpin,
{
    while stream.next().await.is_some() {}
}
