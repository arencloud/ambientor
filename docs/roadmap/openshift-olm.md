# OpenShift OLM / SCC / MemberRoll wizard (Phase 4.4)

## Goal

Provide an OpenShift/OSSM preflight wizard that validates OLM operator install, operator SCC bindings, and ServiceMeshMemberRoll enrollment, with exportable MemberRoll YAML.

## Delivered

- [x] `openshift_wizard` — OLM Subscription/CSV, SCC binding check, MemberRoll merge + manifest
- [x] OSSM `preflight_checks` delegates to wizard steps
- [x] `GET /api/v1/openshift/wizard` — query `enroll`, `ambientorNamespace`, `operatorServiceAccount`
- [x] `ambientor openshift wizard` — direct cluster or via API
- [x] Helm RBAC for `operators.coreos.com` and `security.openshift.io`
- [x] Auto-detect mesh-labeled namespaces missing from MemberRoll when `enroll` omitted

## Branch

Merged via PR [#23](https://github.com/arencloud/ambientor/pull/23).

## Notes

- SCC check looks for `anyuid`, `privileged`, `istio-cni`, or `istio-ingressgateway` among matched SCCs.
- MemberRoll example uses `istio-system` / `default` — adjust for your SMCP namespace before apply.
