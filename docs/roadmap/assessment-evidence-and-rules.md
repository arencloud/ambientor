# Assessment evidence and sidecar rules

Branch: `feature/assessment-evidence-and-rules`

## Goal

Make assessment findings actionable: scan real pod specs for sidecar dependencies, collect DestinationRules, and attach evidence snippets to CR/API status.

## Tasks

### Done in this branch

- [x] `Finding.evidence` field (types + CRD + rules)
- [x] Pod workload scan: localhost `15000/15001`, `holdApplicationUntilProxyStarts`
- [x] `LocalhostProxyRule` and `HoldUntilProxyRule` use `WorkloadContext`
- [x] DestinationRule list + `traffic.destination-rule-subsets` rule
- [x] Evidence on policy/traffic/sidecar findings
- [x] Unit and fixture tests

### Next (follow-up branches)

Track all phases: [PROGRESS.md](../PROGRESS.md).

- [ ] Portal assessment UI to display evidence
- [ ] SARIF export including evidence
- [ ] SPIRE / EnvoyFilter-on-waypoint blockers
- [ ] Namespace-scoped inventory for OSSM member roll
