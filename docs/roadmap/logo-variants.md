# Logo variants for UI (Phase 5.5)

## Goal

Provide reusable logo sizes for the portal, favicon, and documentation, and wire the web UI to serve them.

## Delivered

- [x] Icon variants 32 / 64 / 256 px (`docs/images/logo/`)
- [x] `scripts/generate-logo-variants.sh` — regenerate from `ambientor.png`
- [x] Portal header + favicon (`crates/ambientor-web/assets/logo/`, routes in `ambientor-web`)
- [x] `docs/images/logo/README.md`

## Branch

Merged via PR [#28](https://github.com/arencloud/ambientor/pull/28).

## Files

| Asset | Path |
|-------|------|
| Master wordmark | `docs/images/logo/ambientor.png` |
| Portal icon | `/logo/icon-64.png` (embedded in binary) |
| Favicon | `/favicon.ico` |
