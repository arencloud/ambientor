# ADR 001: In-cluster deployment

## Status

Accepted

## Context

Ambientor must manage Istio/OSSM resources with low latency and without exposing cluster credentials externally.

## Decision

Deploy operator, API, web, and Postgres via Helm in `ambientor-system`. Use in-cluster ServiceAccount for Kubernetes API access. Multi-cluster hub mode uses `ClusterConnection` CRs with secret references.

## Consequences

- Requires cluster-admin or scoped ClusterRole for installation
- Portal users authenticate to Ambientor, not directly to Kubernetes
