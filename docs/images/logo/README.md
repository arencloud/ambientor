# Ambientor logo assets

| File | Use |
|------|-----|
| `ambientor.png` | Full wordmark + emblem (README, docs, marketing) |
| `ambientor-icon.png` | Default icon (64×64 emblem) |
| `ambientor-icon-32.png` | Favicon source |
| `ambientor-icon-64.png` | Portal header |
| `ambientor-icon-256.png` | High-DPI / app icon |

Regenerate icons from the master PNG:

```bash
./scripts/generate-logo-variants.sh
```

Portal serves `crates/ambientor-web/assets/logo/` (`icon-64.png`, `icon-256.png`, `favicon.ico`).
