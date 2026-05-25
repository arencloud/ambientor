# Quay multi-arch images (Phase 5.1)

## Goal

Publish operator, API, web, and CLI container images to Quay for `linux/amd64` and `linux/arm64`, matching Helm image references.

## Delivered

- [x] `.github/workflows/images.yml` — buildx push on `main`, `v*` tags, and `workflow_dispatch`
- [x] `.dockerignore` — smaller build context
- [x] Tags: `quay.io/arencloud/ambientor:<version>-{operator|api|web|cli}`

## Branch

Merged via PR [#24](https://github.com/arencloud/ambientor/pull/24); registry migrated from GHCR to Quay.

## CI secrets

Add repository secrets for push access to `quay.io/arencloud`:

| Secret | Value |
|--------|--------|
| `QUAY_USERNAME` | Quay user or robot account name |
| `QUAY_PASSWORD` | Quay password or robot token |

Create a robot account under the `arencloud` organization with **Write** on the `ambientor` repository.

## Image tags

Helm (`deploy/helm/ambientor/values.yaml`) uses:

```text
quay.io/arencloud/ambientor:0.1.0-operator
quay.io/arencloud/ambientor:0.1.0-api
quay.io/arencloud/ambientor:0.1.0-web
```

CLI image (`0.1.0-cli`) is published for jobs and debugging; not used by the chart by default.

## Pull

Make the repository public (or configure `imagePullSecrets`) and install:

```bash
helm install ambientor deploy/helm/ambientor/ -n ambientor-system --create-namespace \
  --set image.repository=quay.io/arencloud/ambientor \
  --set image.tag=0.1.0
```

## Local build

```bash
./scripts/lab-build-images.sh
# AMBIENTOR_IMAGE_REPO=quay.io/arencloud/ambientor ./scripts/lab-build-images.sh
```
