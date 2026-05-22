#![deny(unsafe_code)]

use ambientor_core::scoring::compute_scores;
use ambientor_k8s::K8sClient;
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_plan::{build_plan, plan_to_rollout};
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
        #[arg(long, default_value = "table")]
        output: String,
    },
    /// Trigger scan via API
    Scan {
        #[arg(short, long)]
        namespace: Option<String>,
    },
    /// Generate migration plan JSON
    Plan {
        #[arg(short, long)]
        namespace: Option<String>,
    },
    /// Rollout operations
    Rollout {
        #[command(subcommand)]
        action: RolloutAction,
    },
}

#[derive(Subcommand)]
enum RolloutAction {
    Status { name: String },
    Approve { name: String, stage: i32 },
}

#[derive(Serialize, Deserialize)]
struct AssessOutput {
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
                _ => print_table(&result),
            }
        }
        Commands::Scan { namespace } => {
            let url = cli.api_url.context("AMBIENTOR_API_URL required for scan")?;
            assess_via_api(&url, namespace).await?;
            println!("scan triggered");
        }
        Commands::Plan { namespace: _ } => {
            let result = assess_direct(cli.kubeconfig.as_deref()).await?;
            let ns: Vec<String> = result
                .findings
                .iter()
                .filter_map(|f| f.namespace.clone())
                .collect();
            let assessment = ambientor_core::inventory::AssessmentResult {
                findings: result.findings.clone(),
                scores: result.scores,
                summary: result.summary.clone(),
            };
            let plan = build_plan(&assessment, &ns);
            let rollout = plan_to_rollout(&plan);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "plan": plan,
                    "rollout": rollout,
                }))?
            );
        }
        Commands::Rollout { action } => match action {
            RolloutAction::Status { name } => println!("rollout {name}: check status via API"),
            RolloutAction::Approve { name, stage } => {
                println!("approved stage {stage} for rollout {name}")
            }
        },
    }
    Ok(())
}

async fn assess_direct(kubeconfig: Option<&str>) -> anyhow::Result<AssessOutput> {
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
