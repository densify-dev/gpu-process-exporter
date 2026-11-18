//! Differential parity harness: runs a reference and a candidate exporter side by side against fake NVML,
//! a hermetic kube-apiserver, and a fake `/proc`, and compares what they expose.
mod env;
mod exporter;
mod runner;
mod scenario;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use parity_harness::{compare, expo};
use runner::{Reference, Settings, Shared};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, ValueEnum)]
enum ReferenceKind {
    Go,
    Golden,
}

#[derive(Parser)]
struct Args {
    /// Reference implementation: the recorded Go goldens, or a live Go binary built from the reference commit.
    #[arg(long, value_enum, default_value = "golden")]
    reference: ReferenceKind,
    /// Go reference binary, used with `--reference=go`.
    #[arg(long, default_value = "target/parity/gpu-exporter-go")]
    reference_bin: PathBuf,
    /// Candidate exporter binary.
    #[arg(long, default_value = "target/debug/gpu-exporter")]
    candidate: PathBuf,
    /// Record reference outputs into the golden directory.
    #[arg(long)]
    record: bool,
    /// Fake NVML library (`libnvidia_ml.so`).
    #[arg(long, default_value = "target/debug/libnvidia_ml.so")]
    fake_nvml: PathBuf,
    /// Directory with etcd and kube-apiserver (as installed by setup-envtest).
    #[arg(long, env = "KUBEBUILDER_ASSETS")]
    envtest: PathBuf,
    #[arg(long, default_value = "parity/harness/scenarios")]
    scenarios: PathBuf,
    #[arg(long, default_value = "parity/golden")]
    golden: PathBuf,
    #[arg(long, default_value = "target/parity")]
    work_dir: PathBuf,
    /// Only run scenarios whose name contains one of these strings.
    #[arg(long = "scenario")]
    filter: Vec<String>,
    /// Repeat the whole suite this many times; every run must pass.
    #[arg(long, default_value_t = 1)]
    runs: usize,
    /// Scenarios run concurrently (exclusive scenarios always run alone).
    #[arg(long, default_value_t = 4)]
    jobs: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let reference = match args.reference {
        ReferenceKind::Go => Reference::Go(
            args.reference_bin
                .canonicalize()
                .context("reference binary")?,
        ),
        ReferenceKind::Golden if args.record => bail!("--record needs --reference=go"),
        ReferenceKind::Golden => Reference::Golden,
    };
    let lib_dir = args.work_dir.join("lib");
    std::fs::create_dir_all(&lib_dir)?;
    std::fs::copy(&args.fake_nvml, lib_dir.join("libnvidia-ml.so.1")).context("copy fake NVML")?;
    let scenarios: Vec<_> = scenario::load_all(&args.scenarios)?
        .into_iter()
        .enumerate()
        .filter(|(_, s)| {
            args.filter.is_empty() || args.filter.iter().any(|f| s.name.contains(f.as_str()))
        })
        .collect();
    let (apiserver, kubeconfigs) =
        env::ApiServer::start(&args.envtest, &args.work_dir.join("apiserver")).await?;
    let admin = env::client(&kubeconfigs.admin).await?;
    let shared = Arc::new(Shared {
        settings: Settings {
            reference,
            candidate: args.candidate.canonicalize().context("candidate binary")?,
            record: args.record,
            golden_dir: args.golden,
            work_dir: args.work_dir.canonicalize()?,
            lib_dir: lib_dir.canonicalize()?,
            kubeconfigs,
        },
        apiserver: tokio::sync::Mutex::new(apiserver),
        admin,
    });
    let scenarios = Arc::new(scenarios);
    let mut failed_runs = 0;
    for run in 1..=args.runs {
        eprintln!("== run {run}/{}", args.runs);
        let failures = run_suite(&shared, &scenarios, args.jobs).await;
        if failures > 0 {
            failed_runs += 1;
        }
        eprintln!(
            "== run {run}: {} passed, {failures} failed",
            scenarios.len() - failures
        );
    }
    if failed_runs > 0 {
        bail!("{failed_runs} of {} runs failed", args.runs);
    }
    Ok(())
}

async fn run_suite(
    shared: &Arc<Shared>,
    scenarios: &Arc<Vec<(usize, scenario::Scenario)>>,
    jobs: usize,
) -> usize {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(jobs.max(1)));
    let mut handles = Vec::new();
    let (exclusive, parallel): (Vec<usize>, Vec<usize>) =
        (0..scenarios.len()).partition(|&i| scenarios[i].1.exclusive);
    for i in parallel {
        let (shared, scenarios, semaphore) = (shared.clone(), scenarios.clone(), semaphore.clone());
        handles.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.expect("semaphore");
            run_one(&shared, &scenarios[i]).await
        }));
    }
    let mut failures = 0;
    for h in handles {
        failures += usize::from(!h.await.expect("scenario task"));
    }
    for i in exclusive {
        failures += usize::from(!run_one(shared, &scenarios[i]).await);
    }
    failures
}

async fn run_one(shared: &Shared, (index, scenario): &(usize, scenario::Scenario)) -> bool {
    let start = Instant::now();
    let result = runner::run(shared, scenario, *index).await;
    let elapsed = start.elapsed().as_secs_f32();
    match result {
        Ok(()) => eprintln!("PASS {} ({elapsed:.1}s)", scenario.name),
        Err(ref e) => eprintln!("FAIL {} ({elapsed:.1}s): {e:#}", scenario.name),
    }
    result.is_ok()
}
