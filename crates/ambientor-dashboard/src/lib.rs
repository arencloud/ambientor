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
    build_cluster_assessment_from_context, cluster_dashboard_meta, dashboard_from_assessment_run,
};
pub use compute::{aggregate_fleet_summary, build_dashboard, namespace_belongs_to_mesh};
pub use dataplane::{DataplaneMode, derive_dataplane_mode, derive_dataplane_mode_from_stored};
pub use types::*;
