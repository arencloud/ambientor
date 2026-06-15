#!/usr/bin/env bash
# Remove Ambientor from the current OpenShift/kubectl context (fresh start).
set -euo pipefail
cd "$(dirname "$0")/.."

NS="${AMBIENTOR_NS:-ambientor-system}"
RELEASE="${HELM_RELEASE:-ambientor}"
DELETE_CRDS="${AMBIENTOR_DELETE_CRDS:-0}"

log() { echo "[cleanup] $*"; }

command -v oc >/dev/null 2>&1 && KUBE=oc || KUBE=kubectl

log "context: $($KUBE config current-context 2>/dev/null || echo unknown)"
log "namespace: ${NS}  release: ${RELEASE}"

if helm status "${RELEASE}" -n "${NS}" >/dev/null 2>&1; then
  log "helm uninstall ${RELEASE}"
  helm uninstall "${RELEASE}" -n "${NS}" --wait --timeout 10m || true
fi

if $KUBE get namespace "${NS}" >/dev/null 2>&1; then
  log "delete namespace ${NS} (CRs, PVCs, routes)"
  $KUBE delete namespace "${NS}" --wait=true --timeout=10m || true
fi

for cr in ambientassessments migrationplans rollouts meshinventories clusterconnections policytranslations; do
  count="$($KUBE get "${cr}.ambientor.io" -A --no-headers 2>/dev/null | wc -l || true)"
  if [[ "${count}" -gt 0 ]]; then
    log "deleting remaining ${cr}.ambientor.io (${count})"
    $KUBE delete "${cr}.ambientor.io" -A --all --wait=false 2>/dev/null || true
  fi
done

if [[ "${DELETE_CRDS}" == "1" ]]; then
  log "delete Ambientor CRDs (set AMBIENTOR_DELETE_CRDS=0 to keep)"
  $KUBE delete crd \
    ambientassessments.ambientor.io \
    migrationplans.ambientor.io \
    rollouts.ambientor.io \
    meshinventories.ambientor.io \
    clusterconnections.ambientor.io \
    policytranslations.ambientor.io \
    --wait=false 2>/dev/null || true
fi

log "done — cluster ready for fresh install (./scripts/openshift-pilot-install.sh or dev-build-push.sh)"
