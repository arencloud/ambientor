# Lab validation (Phase 1, Step 1.1)

**Status:** ✅ Runbook and automation delivered on `main` (see [runbook-lab.md](../runbook-lab.md)).

## Operator checklist

- [ ] Run `./scripts/lab-kind-up.sh`
- [ ] Install Istio ambient + Gateway API per runbook
- [ ] Deploy bookinfo
- [ ] `./scripts/lab-build-images.sh` && `./scripts/lab-load-kind.sh`
- [ ] Helm install with `values-lab.yaml`
- [ ] Apply `docs/lab/meshinventory-bookinfo.yaml`
- [ ] Complete validation table §10 in runbook
- [ ] Add lab result note to [PROGRESS.md](../PROGRESS.md) step 1.1 Notes

## Next code step

Step **1.4** — [PROGRESS.md](../PROGRESS.md): operator informers.
