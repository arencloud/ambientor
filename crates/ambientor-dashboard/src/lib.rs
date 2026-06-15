#![deny(unsafe_code)]

mod application_types;
mod applications;
mod assess_run;
mod compute;
mod dashboard_from_run;
mod dataplane;
mod deep_analysis;
mod findings_attribution;
mod types;

pub use application_types::*;
pub use applications::{derive_risk_level, discover_ingress_gateway_namespaces, hostnames_by_namespace};
pub use assess_run::{
    build_cluster_assessment_from_context, build_cluster_assessment_from_inventory,
    cluster_dashboard_meta, cluster_dashboard_meta_with_meshes, dashboard_from_assessment_run,
};
pub use compute::{
    aggregate_fleet_summary, build_dashboard, compute_migration_savings_from_dashboard,
    namespace_belongs_to_mesh, AssessmentFindingsOverrides,
};
pub use dashboard_from_run::compute_migration_savings;
pub use dataplane::{
    DataplaneMode, derive_dataplane_mode, derive_dataplane_mode_from_stored,
    is_migration_candidate, namespace_is_migrated,
};
pub use types::*;
