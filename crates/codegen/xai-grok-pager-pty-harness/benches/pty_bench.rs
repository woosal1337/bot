// Modified by the Bot project on 2026-09-13: Require complete measured baselines.
//! Spawns the real pager binary in a PTY, dispatches named scenarios, and emits aggregated results as JSON.
//! Supports baseline comparison for CI regression detection.
//!
//! ## Typical use
//!
//! Run a single scenario locally:
//! ```bash
//! cargo bench -p xai-grok-pager-pty-harness \
//!   --bench pty_bench -- --scenario scroll-stress
//! ```
//!
//! Run every scenario and write a new baseline:
//! ```bash
//! cargo bench -p xai-grok-pager-pty-harness \
//!   --bench pty_bench -- --all \
//!   --write-baseline benches/pty_baselines/local.json
//! ```
//!
//! Run every scenario in CI and fail on >15% p99 regression:
//! ```bash
//! PAGER_BINARY=./artifacts/grok-${VERSION}-linux-x86_64 \
//!   cargo bench -p xai-grok-pager-pty-harness \
//!   --bench pty_bench -- --all \
//!   --baseline benches/pty_baselines/linux-x86_64.json
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser as ClapParser;
use xai_grok_pager_pty_harness::{
    BenchEnvironment, BenchReport, BenchResults, ContentController, PtyHarness, Scenario,
    baseline_environment_mismatches, compare_baseline, incomplete_scenarios,
    invalid_baseline_scenarios, pager_binary,
    results::{DEFAULT_REGRESSION_THRESHOLD, load_baseline, write_baseline},
};

#[derive(ClapParser, Debug)]
#[command(
    name = "pty-bench",
    about = "Measure Bot terminal performance in a real PTY",
    long_about = None,
)]
struct Cli {
    /// Run a single scenario by name. Mutually exclusive with --all.
    #[arg(long, value_enum, conflicts_with = "all")]
    scenario: Option<Scenario>,

    /// Run every scenario.
    #[arg(long)]
    all: bool,

    /// Path to the pager binary.
    /// Defaults to auto-resolve (PAGER_BINARY env or a locally-built debug binary).
    #[arg(long)]
    binary: Option<PathBuf>,

    /// Terminal rows.
    #[arg(long, default_value_t = 50)]
    rows: u16,

    /// Terminal columns.
    #[arg(long, default_value_t = 120)]
    cols: u16,

    /// Compare results against a baseline file and exit non-zero on regression (>15% p99 delta by default).
    #[arg(long, value_name = "PATH")]
    baseline: Option<PathBuf>,

    /// Save the current run as a new baseline.
    #[arg(long, value_name = "PATH", conflicts_with = "baseline")]
    write_baseline: Option<PathBuf>,

    /// Regression threshold as a fraction of baseline p99 (0.15 = 15%).
    #[arg(long, default_value_t = DEFAULT_REGRESSION_THRESHOLD)]
    threshold: f64,

    #[arg(long, help = "Machine name stored in the report")]
    machine: Option<String>,

    #[arg(
        long,
        default_value = "xterm-256color",
        help = "TERM profile used by the Bot child and stored in the report"
    )]
    terminal: String,

    #[arg(long, help = "Binary build profile stored in the report")]
    build_profile: Option<String>,

    /// Accepted for `cargo bench` compatibility (libtest-style argument).
    /// We ignore it; this isn't a libtest harness.
    #[arg(long, hide = true)]
    #[allow(dead_code)]
    bench: bool,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    match run().await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("pty-bench failed: {e:#}");
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    let binary = match &cli.binary {
        Some(b) => b.clone(),
        None => pager_binary().context("resolve pager binary")?,
    };
    let binary = dunce::canonicalize(&binary)
        .with_context(|| format!("resolve benchmark binary {}", binary.display()))?;
    let environment = benchmark_environment(&cli, &binary)?;
    let scenarios: Vec<Scenario> = if cli.all {
        Scenario::ALL.to_vec()
    } else if let Some(s) = cli.scenario {
        vec![s]
    } else {
        bail!("specify --scenario <name> or --all");
    };

    tracing::info!(
        binary = %binary.display(),
        rows = cli.rows,
        cols = cli.cols,
        count = scenarios.len(),
        "starting pty-bench run"
    );

    let mut results: Vec<BenchResults> = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        tracing::info!(scenario = scenario.as_ref(), "running scenario");
        let content = ContentController::start()
            .await
            .context("start ContentController")?;
        let mut harness = PtyHarness::spawn_with_content_env(
            &binary,
            cli.rows,
            cli.cols,
            &content,
            &[],
            &[("TERM", environment.terminal.as_str())],
        )
        .context("spawn pager PTY harness")?;

        let res = scenario.run(&mut harness, &content).await;

        // Best-effort cleanup regardless of scenario outcome.
        let _ = harness.quit();

        match res {
            Ok(r) => {
                tracing::info!(
                    scenario = %r.scenario,
                    frames = r.total_frames,
                    p50_ms = r.p50_ms,
                    p99_ms = r.p99_ms,
                    "scenario complete"
                );
                results.push(r);
            }
            Err(e) => {
                tracing::warn!(scenario = scenario.as_ref(), error = %e, "scenario failed");
                results.push(BenchResults::failed(scenario.as_ref()));
            }
        }
    }

    let report = BenchReport::new(environment, results);
    let json = serde_json::to_string_pretty(&report).context("serialize results")?;
    println!("{json}");

    let incomplete = incomplete_scenarios(&report.results);
    if !incomplete.is_empty() {
        eprintln!(
            "FAILED: {} scenario(s) did not complete: {}",
            incomplete.len(),
            incomplete.join(", ")
        );
        return Ok(ExitCode::from(1));
    }

    if let Some(path) = cli.write_baseline {
        write_baseline(&path, &report)?;
        eprintln!("wrote baseline to {}", path.display());
    }

    if let Some(path) = cli.baseline {
        let baseline = load_baseline(&path)?;
        let mismatches =
            baseline_environment_mismatches(&report.environment, &baseline.environment);
        if !mismatches.is_empty() {
            eprintln!("FAILED: baseline environment does not match this run:");
            for mismatch in mismatches {
                eprintln!("  {mismatch}");
            }
            return Ok(ExitCode::from(1));
        }
        let invalid = invalid_baseline_scenarios(&report.results, &baseline.results);
        if !invalid.is_empty() {
            eprintln!(
                "FAILED: missing or invalid baseline data for {} scenario(s): {}",
                invalid.len(),
                invalid.join(", ")
            );
            return Ok(ExitCode::from(1));
        }
        let regressions = compare_baseline(&report.results, &baseline.results, cli.threshold);
        if regressions.is_empty() {
            eprintln!(
                "OK: no scenarios regressed beyond {:.0}% of baseline p99",
                cli.threshold * 100.0
            );
        } else {
            eprintln!(
                "REGRESSION: {} scenario(s) exceeded {:.0}% p99 threshold:",
                regressions.len(),
                cli.threshold * 100.0
            );
            for r in &regressions {
                eprintln!(
                    "  {}: {:.2}ms -> {:.2}ms ({:+.1}%)",
                    r.scenario,
                    r.baseline_p99_ms,
                    r.current_p99_ms,
                    r.pct_delta * 100.0
                );
            }
            return Ok(ExitCode::from(1));
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn benchmark_environment(cli: &Cli, binary: &std::path::Path) -> Result<BenchEnvironment> {
    let machine = cli
        .machine
        .clone()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .context("detect machine name; pass --machine")?;
    let terminal = cli.terminal.trim().to_owned();
    if terminal.is_empty() {
        bail!("terminal must not be empty");
    }
    let build_profile = cli
        .build_profile
        .clone()
        .or_else(|| infer_build_profile(binary))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .context("detect binary build profile; pass --build-profile")?;
    Ok(BenchEnvironment {
        machine,
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        terminal,
        rows: cli.rows,
        cols: cli.cols,
        build_profile,
        binary: binary.display().to_string(),
    })
}

fn infer_build_profile(binary: &std::path::Path) -> Option<String> {
    binary.components().rev().find_map(|component| {
        let component = component.as_os_str().to_string_lossy();
        matches!(component.as_ref(), "debug" | "release" | "bench").then(|| component.into_owned())
    })
}
