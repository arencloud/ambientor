use std::collections::BTreeSet;

use ambientor_types::{Finding, LabelSelector};
use k8s_openapi::api::core::v1::Namespace;
use kube::{Api, Client, api::ListParams};

/// Namespace list for wave planning (defaults to `default` when no sources).
pub fn namespaces_for_planning(findings: &[Finding], inventory_namespaces: &[String]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for f in findings {
        if let Some(ns) = &f.namespace {
            set.insert(ns.clone());
        }
    }
    for ns in inventory_namespaces {
        if !ns.is_empty() {
            set.insert(ns.clone());
        }
    }
    if set.is_empty() {
        return vec!["default".into()];
    }
    if set.len() > 1 {
        set.remove("default");
    }
    set.into_iter().collect()
}

/// Namespaces whose labels satisfy `namespaceSelector.matchLabels`.
pub async fn namespaces_matching_selector(
    client: &Client,
    selector: &Option<LabelSelector>,
) -> Result<Vec<String>, kube::Error> {
    let Some(selector) = selector else {
        return Ok(Vec::new());
    };
    let required = match selector.match_labels.as_ref() {
        Some(labels) if !labels.is_empty() => labels,
        _ => return Ok(Vec::new()),
    };

    let api: Api<Namespace> = Api::all(client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .filter_map(|ns| {
            let name = ns.metadata.name?;
            let labels = ns.metadata.labels.as_ref()?;
            if required
                .iter()
                .all(|(key, value)| labels.get(key).is_some_and(|v| v == value))
            {
                Some(name)
            } else {
                None
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_inventory_over_default_fallback() {
        let ns = namespaces_for_planning(&[], &["bookinfo".into()]);
        assert_eq!(ns, vec!["bookinfo"]);
    }

    #[test]
    fn drops_default_when_other_namespaces_present() {
        let ns = namespaces_for_planning(
            &[Finding {
                namespace: Some("default".into()),
                ..finding_stub()
            }],
            &["bookinfo".into()],
        );
        assert_eq!(ns, vec!["bookinfo"]);
    }

    fn finding_stub() -> Finding {
        Finding {
            id: "test".into(),
            severity: ambientor_types::FindingSeverity::Info,
            category: ambientor_types::FindingCategory::Readiness,
            title: "t".into(),
            message: "m".into(),
            namespace: None,
            resource: None,
            remediation: None,
            doc_url: None,
            evidence: None,
        }
    }
}
