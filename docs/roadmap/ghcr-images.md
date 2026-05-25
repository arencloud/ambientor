# GHCR multi-arch images (Phase 5.1)

## Goal

Publish operator, API, web, and CLI container images to GitHub Container Registry for `linux/amd64` and `linux/arm64`, matching Helm image references.

## Delivered

- [x] `.github/workflows/images.yml` — buildx push on `main`, `v*` tags, and `workflow_dispatch`
- [x] `.dockerignore` — smaller build context
- [x] Tags: `ghcr.io/arencloud/ambientor:<version>-{operator|api|web|cli}`

## Branch

`feature/ghcr-images`

## Image tags

Helm (`deploy/helm/ambientor/values.yaml`) uses:

```text
ghcr.io/arencloud/ambientor:0.1.0-operator
ghcr.io/arencloud/ambientor:0.1.0-api
ghcr.io/arencloud/ambientor:0.1.0-web
```

CLI image (`0.1.0-cli`) is published for jobs and debugging; not used by the chart by default.

## Pull

After the workflow runs, make the package public (or configure `imagePullSecrets`) and install:

```bash
helm install ambientor deploy/helm/ambientor/ -n ambientor-system --create-namespace \
  --set image.repository=ghcr.io/arencloud/ambientor \
  --set image.tag=0.1.0
```

## Local build

```bash
./scripts/lab-build-images.sh
# AMBIENTOR_IMAGE_REPO=ghcr.io/arencloud/ambientor ./scripts/lab-build-images.sh
```
