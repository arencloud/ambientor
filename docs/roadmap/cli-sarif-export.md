# CLI SARIF export (Phase 1, Step 1.8)

Branch: `feature/cli-sarif-export`

## Goal

Export assessment findings as SARIF 2.1.0 for CI gates and security scanners (GitHub Code Scanning, etc.).

## Delivered

- [x] `ambientor assess --output sarif` (direct cluster or via `AMBIENTOR_API_URL`)
- [x] Rule catalog from finding IDs; results include evidence, remediation, namespace
- [x] Run-level properties: scores and summary

## Usage

```bash
ambientor assess --output sarif > ambientor.sarif.json
ambientor assess --output sarif --kubeconfig ~/.kube/config
```

## Test plan

- [x] Unit tests in `ambientor-cli/src/sarif.rs`
- [x] `cargo test --workspace`
