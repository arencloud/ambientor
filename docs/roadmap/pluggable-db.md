# Pluggable DB trait (Phase 5.4)

## Goal

Decouple API and operator persistence from concrete Postgres repository types so alternate backends can be added without changing call sites.

## Delivered

- [x] `ScanStore`, `AuditStore`, `UserStore` async traits (`ambientor-db`)
- [x] Postgres implementations on existing repositories
- [x] `open_postgres()` → `DbBackend` with `Arc<dyn …>` store handles
- [x] API and operator wired through trait objects

## Branch

`feature/pluggable-db`

## Usage

```rust
let db = ambientor_db::open_postgres(&database_url).await?;
db.scan.record_completed(cluster_ref, ns, &payload).await?;
db.audit.append(&event).await?;
```

`connect` / `migrate` remain available for low-level access; prefer `open_postgres` for services.

## Future backends

Implement the three traits (e.g. SQLite, cloud DSN) and construct `DbBackend` with the new `Arc<dyn ScanStore>` implementations.
