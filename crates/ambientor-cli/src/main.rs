#![deny(unsafe_code)]

mod mesh_cmd;
mod plan_cmd;
mod rollout_cmd;
mod sarif;

use ambientor_core::scoring::compute_scores;
use ambientor_k8s::K8sClient;
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_mesh::{OpenShiftWizardOptions, namespaces_needing_enrollment, run_wizard};
use ambientor_scan::default_registry;
use ambientor_types::FindingSummary;
use anyhow::Context;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(name = "ambientor", about = "Ambient Mesh Migration Assistant")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// API base URL when using remote mode
    #[arg(long, env = "AMBIENTOR_API_URL")]
    api_url: Option<String>,
    /// Path to kubeconfig for direct cluster access
    #[arg(long, env = "KUBECONFIG")]
    kubeconfig: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run full mesh assessment
    Assess {
        #[arg(short, long)]
        namespace: Option<String>,
        /// Output format: table, json, or sarif
        #[arg(long, default_value = "table")]
        output: String,
    },
    /// Trigger scan via API
    Scan {
        #[arg(short, long)]
        namespace: Option<String>,
    },
    /// Migration plan commands
    Plan {
        #[command(subcommand)]
        action: PlanAction,
    },
    /// Rollout operations
    Rollout {
        #[command(subcommand)]
        action: RolloutAction,
    },
    /// OpenShift / OSSM preflight wizard (OLM, SCC, MemberRoll)
    Openshift {
        #[command(subcommand)]
        action: OpenshiftAction,
    },
    /// Istio / OSSM control-plane discovery and namespace enrollment
    Mesh {
        #[command(subcommand)]
        action: MeshAction,
    },
}

#[derive(Subcommand)]
enum MeshAction {
    /// List mesh instances (istiod revisions + enrollment contracts)
    Instances,
    /// Enroll a namespace on a mesh target (labels + OSSM MemberRoll when applicable)
    Enroll {
        #[arg(short, long)]
        namespace: String,
        #[arg(long)]
        revision: Option<String>,
        #[arg(long)]
        discovery_label: Option<String>,
        #[arg(long)]
        control_plane_namespace: Option<String>,
    },
}

#[derive(Subcommand)]
enum OpenshiftAction {
    /// Run OLM + SCC + MemberRoll wizard
    Wizard {
        /// Comma-separated namespaces to enroll in MemberRoll suggestion
        #[arg(long)]
        enroll: Option<String>,
        #[arg(long, default_value = "ambientor-system")]
        ambientor_namespace: String,
        #[arg(long, default_value = "ambientor-operator")]
        operator_service_account: String,
    },
}

#[derive(Subcommand)]
enum PlanAction {
    /// Run assessment and build a migration plan (optionally write GitOps bundle)
    Create {
        #[arg(short, long)]
        namespace: Option<String>,
        /// Write `migration-bundle.yaml` and `plan.json` to this directory
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Print plan JSON to stdout (default when --out is omitted)
        #[arg(long)]
        json: bool,
    },
    /// Export an existing cluster MigrationPlan as a YAML bundle
    Export {
        #[arg(short, long)]
        namespace: String,
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        out: Option<std::path::PathBuf>,
    },
    /// Mark plan approved (`status.approved=true`) — same as GitOps patch
    Approve {
        #[arg(short, long, default_value = "default")]
        namespace: String,
        #[arg(short, long)]
        name: String,
    },
    /// One approval: approve plan, create rollout, approve stage 0
    Execute {
        #[arg(short, long, default_value = "default")]
        namespace: String,
        #[arg(short, long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum RolloutAction {
    /// Show rollout status (API or kube)
    Status {
        #[arg(short, long, default_value = "default")]
        namespace: String,
        #[arg(short, long)]
        name: String,
    },
    /// Approve the current stage (`approvedStage` patch)
    Approve {
        #[arg(short, long, default_value = "default")]
        namespace: String,
        #[arg(short, long)]
        name: String,
        /// Stage index (defaults to current stage)
        #[arg(short, long)]
        stage: Option<i32>,
    },
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AssessOutput {
    findings: Vec<ambientor_types::Finding>,
    scores: ambientor_types::AssessmentScores,
    summary: FindingSummary,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Assess { namespace, output } => {
            let result = if let Some(url) = cli.api_url {
                assess_via_api(&url, namespace).await?
            } else {
                assess_direct(cli.kubeconfig.as_deref()).await?
            };
            match output.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&result)?),
                "sarif" => {
                    let doc = sarif::to_sarif(&result);
                    println!("{}", serde_json::to_string_pretty(&doc)?);
                }
                _ => print_table(&result),
            }
        }
        Commands::Scan { namespace } => {
            let url = cli.api_url.context("AMBIENTOR_API_URL required for scan")?;
            assess_via_api(&url, namespace).await?;
            println!("scan triggered");
        }
        Commands::Plan { action } => match action {
            PlanAction::Create {
                namespace,
                out,
                json,
            } => {
                plan_cmd::plan_create(cli.kubeconfig.as_deref(), namespace, out, json).await?;
            }
            PlanAction::Export {
                namespace,
                name,
                out,
            } => {
                plan_cmd::plan_export(
                    cli.kubeconfig.as_deref(),
                    cli.api_url.as_deref(),
                    namespace,
                    name,
                    out,
                )
                .await?;
            }
            PlanAction::Approve { namespace, name } => {
                plan_cmd::plan_approve(cli.api_url.as_deref(), cli.kubeconfig.as_deref(), &namespace, &name)
                    .await?;
            }
            PlanAction::Execute { namespace, name } => {
                plan_cmd::plan_execute(cli.api_url.as_deref(), cli.kubeconfig.as_deref(), &namespace, &name)
                    .await?;
            }
        },
        Commands::Openshift { action } => match action {
            OpenshiftAction::Wizard {
                enroll,
                ambientor_namespace,
                operator_service_account,
            } => {
                let report = if let Some(url) = cli.api_url.as_deref() {
                    openshift_wizard_via_api(
                        url,
                        enroll.as_deref(),
                        &ambientor_namespace,
                        &operator_service_account,
                    )
                    .await?
                } else {
                    openshift_wizard_direct(
                        cli.kubeconfig.as_deref(),
                        enroll.as_deref(),
                        &ambientor_namespace,
                        &operator_service_account,
                    )
                    .await?
                };
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        },
        Commands::Mesh { action } => match action {
            MeshAction::Instances => {
                mesh_cmd::list_mesh_instances(cli.kubeconfig.as_deref()).await?;
            }
            MeshAction::Enroll {
                namespace,
                revision,
                discovery_label,
                control_plane_namespace,
            } => {
                let target = ambientor_types::MeshTarget {
                    revision,
                    discovery_label,
                    control_plane_namespace,
                };
                mesh_cmd::enroll_namespace(cli.kubeconfig.as_deref(), &namespace, target).await?;
            }
        },
        Commands::Rollout { action } => match action {
            RolloutAction::Status { namespace, name } => {
                rollout_cmd::rollout_status(
                    cli.api_url.as_deref(),
                    cli.kubeconfig.as_deref(),
                    &namespace,
                    &name,
                )
                .await?;
            }
            RolloutAction::Approve {
                namespace,
                name,
                stage,
            } => {
                rollout_cmd::rollout_approve(
                    cli.api_url.as_deref(),
                    cli.kubeconfig.as_deref(),
                    &namespace,
                    &name,
                    stage,
                )
                .await?;
            }
        },
    }
    Ok(())
}

pub(crate) async fn assess_direct(kubeconfig: Option<&str>) -> anyhow::Result<AssessOutput> {
    let k8s = match kubeconfig {
        Some(p) => K8sClient::from_kubeconfig(Some(p)).await?,
        None => K8sClient::in_cluster()
            .await
            .or(K8sClient::from_kubeconfig(None).await)?,
    };
    let platform = ambientor_k8s::detect_platform(&k8s.client).await?;
    let backend = backend_for_flavor(platform.mesh_flavor);
    let ctx = backend.build_rule_context(&k8s.client).await?;
    let findings = default_registry().evaluate_all(&ctx);
    let scores = compute_scores(&findings);
    let summary = FindingSummary::from_findings(&findings);
    Ok(AssessOutput {
        findings,
        scores,
        summary,
    })
}

async fn assess_via_api(base: &str, namespace: Option<String>) -> anyhow::Result<AssessOutput> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/v1/assess"))
        .json(&serde_json::json!({ "namespace": namespace }))
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}

async fn openshift_wizard_direct(
    kubeconfig: Option<&str>,
    enroll: Option<&str>,
    ambientor_namespace: &str,
    operator_service_account: &str,
) -> anyhow::Result<ambientor_mesh::OpenShiftWizardReport> {
    let k8s = match kubeconfig {
        Some(p) => K8sClient::from_kubeconfig(Some(p)).await?,
        None => K8sClient::in_cluster()
            .await
            .or(K8sClient::from_kubeconfig(None).await)?,
    };
    let platform = ambientor_k8s::detect_platform(&k8s.client).await?;
    let mut enroll_ns: Vec<String> = enroll
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if enroll_ns.is_empty() {
        enroll_ns = namespaces_needing_enrollment(&k8s.client).await?;
    }
    let opts = OpenShiftWizardOptions {
        ambientor_namespace: ambientor_namespace.into(),
        operator_service_account: operator_service_account.into(),
        enroll_namespaces: enroll_ns,
    };
    run_wizard(&k8s.client, &platform, &opts).await
}

async fn openshift_wizard_via_api(
    base: &str,
    enroll: Option<&str>,
    ambientor_namespace: &str,
    operator_service_account: &str,
) -> anyhow::Result<ambientor_mesh::OpenShiftWizardReport> {
    let client = reqwest::Client::new();
    let url = format!("{base}/api/v1/openshift/wizard");
    let mut params = vec![
        ("ambientorNamespace", ambientor_namespace),
        ("operatorServiceAccount", operator_service_account),
    ];
    if let Some(e) = enroll {
        params.push(("enroll", e));
    }
    let resp = client
        .get(&url)
        .query(&params)
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}

fn print_table(out: &AssessOutput) {
    println!("Overall score: {}", out.scores.overall);
    println!(
        "Blockers: {}  Warnings: {}  Info: {}",
        out.summary.blockers, out.summary.warnings, out.summary.info
    );
    for f in &out.findings {
        println!("[{:?}] {} — {}", f.severity, f.title, f.message);
    }
}
