use ambientor_analyze::traffic::register_traffic_rules;
use ambientor_core::rules::RuleRegistry;

use crate::readiness::{
    AmbientComponentsRule, GatewayApiRule, PeerAuthDisableRule, VmWorkloadRule,
};
use crate::sidecar::{HoldUntilProxyRule, LocalhostProxyRule};

pub fn default_registry() -> RuleRegistry {
    let mut registry = RuleRegistry::new();
    registry.register(Box::new(GatewayApiRule));
    registry.register(Box::new(AmbientComponentsRule));
    registry.register(Box::new(PeerAuthDisableRule));
    registry.register(Box::new(VmWorkloadRule));
    registry.register(Box::new(LocalhostProxyRule));
    registry.register(Box::new(HoldUntilProxyRule));
    register_traffic_rules(&mut registry);
    registry
}
