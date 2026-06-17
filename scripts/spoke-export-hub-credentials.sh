#!/usr/bin/env bash
# Apply spoke hub-reader RBAC (optional) and print a hub ClusterConnection credentials secret.
#
# Usage (on spoke context):
#   ./scripts/spoke-export-hub-credentials.sh
#   ./scripts/spoke-export-hub-credentials.sh --apply
#   KUBECONFIG=spoke.yaml ./scripts/spoke-export-hub-credentials.sh --apply --write-hub-secret ./cl02-hub-secret.yaml
#
# Hub API server URL: set SPOKE_API_SERVER or pass --api-server https://api.example:6443
# (defaults to current kubectl cluster server).
#
set -euo pipefail
cd "$(dirname "$0")/.."

APPLY=false
WRITE_HUB_SECRET=""
API_SERVER="${SPOKE_API_SERVER:-}"
SPOKE_NS="${AMBIENTOR_SPOKE_NS:-ambientor-system}"
SA_NAME="${AMBIENTOR_SPOKE_SA:-ambientor-hub-reader}"
TOKEN_SECRET="${AMBIENTOR_SPOKE_TOKEN_SECRET:-ambientor-hub-reader-token}"

usage() {
  sed -n '2,12p' "$0"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --apply)
      APPLY=true
      shift
      ;;
    --write-hub-secret)
      WRITE_HUB_SECRET="${2:-}"
      shift 2
      ;;
    --write-hub-secret=*)
      WRITE_HUB_SECRET="${1#--write-hub-secret=}"
      shift
      ;;
    --api-server)
      API_SERVER="${2:-}"
      shift 2
      ;;
    --api-server=*)
      API_SERVER="${1#--api-server=}"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! command -v kubectl >/dev/null 2>&1; then
  echo "kubectl is required" >&2
  exit 1
fi

if [[ "$APPLY" == true ]]; then
  kubectl apply -f docs/lab/spoke-hub-reader-rbac.yaml
fi

if [[ -z "$API_SERVER" ]]; then
  API_SERVER="$(kubectl config view --minify -o jsonpath='{.clusters[0].cluster.server}')"
fi
if [[ -z "$API_SERVER" ]]; then
  echo "Could not determine API server; set SPOKE_API_SERVER or pass --api-server" >&2
  exit 1
fi

echo "Waiting for service account token secret ${SPOKE_NS}/${TOKEN_SECRET}..." >&2
token_b64=""
for _ in $(seq 1 60); do
  if kubectl -n "$SPOKE_NS" get secret "$TOKEN_SECRET" >/dev/null 2>&1; then
    token_b64="$(kubectl -n "$SPOKE_NS" get secret "$TOKEN_SECRET" -o jsonpath='{.data.token}' 2>/dev/null || true)"
    if [[ -n "$token_b64" ]]; then
      break
    fi
  fi
  sleep 1
done

ca_b64="$(kubectl -n "$SPOKE_NS" get secret "$TOKEN_SECRET" -o jsonpath='{.data.ca\.crt}' 2>/dev/null || true)"
if [[ -z "$token_b64" || -z "$ca_b64" ]]; then
  echo "Token secret not ready. If your cluster does not auto-populate legacy SA tokens, run:" >&2
  echo "  oc create token ${SA_NAME} -n ${SPOKE_NS} --duration=87600h" >&2
  echo "and build the hub secret manually with token + server + ca.crt." >&2
  exit 1
fi

token="$(printf '%s' "$token_b64" | base64 -d)"
ca_pem="$(printf '%s' "$ca_b64" | base64 -d)"

hub_secret_yaml="$(cat <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: spoke-credentials
  namespace: ambientor-system
type: Opaque
stringData:
  token: |
$(printf '%s\n' "$token" | sed 's/^/    /')
  server: "${API_SERVER}"
  ca.crt: |
$(printf '%s\n' "$ca_pem" | sed 's/^/    /')
EOF
)"

if [[ -n "$WRITE_HUB_SECRET" ]]; then
  printf '%s\n' "$hub_secret_yaml" >"$WRITE_HUB_SECRET"
  echo "Wrote hub credentials secret manifest: $WRITE_HUB_SECRET" >&2
else
  printf '%s\n' "$hub_secret_yaml"
fi

echo "---" >&2
echo "On the hub: oc apply -f <secret> then apply ClusterConnection (see docs/lab/clusterconnection-spoke.example.yaml)" >&2
