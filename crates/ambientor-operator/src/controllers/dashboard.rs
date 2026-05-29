use std::sync::Arc;
use std::time::Duration;

use ambientor_dashboard::build_dashboard;
use ambientor_db::{DashboardStore, cluster_ref_from_env};
use kube::Client;
use tracing::info;

pub async fn run(client: Client, store: Option<Arc<dyn DashboardStore>>) {
    let Some(store) = store else {
        tracing::debug!("DATABASE_URL not set; dashboard sync disabled");
        return;
    };

    info!("dashboard sync loop started (interval 30s)");
    let store = store.clone();
    loop {
        let cluster_ref = cluster_ref_from_env();
        match build_dashboard(&client, &cluster_ref).await {
            Ok(response) => {
                if let Err(e) = store.sync_snapshot(&response).await {
                    tracing::warn!(
                        error = %e,
                        cluster_ref = %cluster_ref,
                        "dashboard sync to database failed"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    cluster_ref = %cluster_ref,
                    "dashboard compute failed"
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}
