// Modified by the Bot project on 2026-09-15: use a neutral measured-baseline fixture.
//! Aggregated benchmark results, percentile computation, baseline compare.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::timing::FrameTiming;

/// Aggregated benchmark results for a single scenario run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResults {
    pub scenario: String,
    pub completed: bool,
    pub total_frames: u64,
    pub avg_fps: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub jank_count: u64,
    pub jank_rate: f64,
    pub chars_per_frame_avg: f64,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BenchEnvironment {
    pub machine: String,
    pub platform: String,
    pub terminal: String,
    pub rows: u16,
    pub cols: u16,
    pub build_profile: String,
    pub binary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    pub schema_version: u32,
    pub environment: BenchEnvironment,
    pub scenario_count: usize,
    pub event_count: u64,
    pub results: Vec<BenchResults>,
}

impl BenchReport {
    pub fn new(environment: BenchEnvironment, results: Vec<BenchResults>) -> Self {
        Self {
            schema_version: 1,
            scenario_count: results.len(),
            event_count: results.iter().map(|result| result.total_frames).sum(),
            environment,
            results,
        }
    }
}

impl BenchResults {
    /// `wall_time` is caller-measured elapsed collection time, used for `avg_fps`.
    pub fn from_timings(scenario: &str, timings: &[FrameTiming], wall_time: Duration) -> Self {
        let total_frames = timings.len() as u64;

        if timings.is_empty() {
            return Self {
                scenario: scenario.to_owned(),
                completed: true,
                total_frames: 0,
                avg_fps: 0.0,
                p50_ms: 0.0,
                p99_ms: 0.0,
                max_ms: 0.0,
                jank_count: 0,
                jank_rate: 0.0,
                chars_per_frame_avg: 0.0,
            };
        }

        let mut durations_ms: Vec<f64> = timings
            .iter()
            .map(|t| t.duration.as_secs_f64() * 1000.0)
            .collect();
        durations_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let wall_secs = wall_time.as_secs_f64();
        let avg_fps = if wall_secs > 0.0 {
            total_frames as f64 / wall_secs
        } else {
            0.0
        };
        let p50_ms = percentile(&durations_ms, 50.0);
        let p99_ms = percentile(&durations_ms, 99.0);
        let max_ms = durations_ms.last().copied().unwrap_or(0.0);

        // Jank threshold: frame time over 2x the median (p50)
        let jank_threshold = p50_ms * 2.0;
        let jank_count = durations_ms.iter().filter(|&&d| d > jank_threshold).count() as u64;
        let jank_rate = jank_count as f64 / total_frames as f64;

        let total_chars: usize = timings.iter().map(|t| t.chars).sum();
        let chars_per_frame_avg = total_chars as f64 / total_frames as f64;

        Self {
            scenario: scenario.to_owned(),
            completed: true,
            total_frames,
            avg_fps,
            p50_ms,
            p99_ms,
            max_ms,
            jank_count,
            jank_rate,
            chars_per_frame_avg,
        }
    }

    pub fn failed(scenario: &str) -> Self {
        let mut result = Self::from_timings(scenario, &[], Duration::ZERO);
        result.completed = false;
        result
    }
}

// ── Baseline comparison ────────────────────────────────────────────────────

/// Regression threshold: fail if a scenario's p99 frame time grows by more than this fraction (0.15 = 15%).
pub const DEFAULT_REGRESSION_THRESHOLD: f64 = 0.15;

pub fn load_baseline(path: &Path) -> Result<BenchReport> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read baseline file {}", path.display()))?;
    let report: BenchReport = serde_json::from_str(&text)
        .with_context(|| format!("parse baseline file {}", path.display()))?;
    validate_report(&report)
        .with_context(|| format!("validate baseline file {}", path.display()))?;
    Ok(report)
}

pub fn write_baseline(path: &Path, report: &BenchReport) -> Result<()> {
    validate_report(report).context("validate baseline")?;
    let json = serde_json::to_string_pretty(report).context("serialize baseline")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create baseline parent {}", parent.display()))?;
    }
    std::fs::write(path, json)
        .with_context(|| format!("write baseline file {}", path.display()))?;
    Ok(())
}

/// Outcome of comparing a single scenario's current run against its baseline.
#[derive(Debug, Clone)]
pub struct ScenarioRegression {
    pub scenario: String,
    pub baseline_p99_ms: f64,
    pub current_p99_ms: f64,
    pub pct_delta: f64,
}

/// Compare the given `results` against `baseline`, returning every scenario whose p99 grew by more than `threshold` (as a fraction, e.g. 0.15 = 15%).
///
/// Scenarios missing from the baseline are skipped (first run of a new scenario is not a regression).
pub fn compare_baseline(
    results: &[BenchResults],
    baseline: &[BenchResults],
    threshold: f64,
) -> Vec<ScenarioRegression> {
    let mut regressions = Vec::new();
    for r in results {
        let Some(b) = baseline.iter().find(|entry| entry.scenario == r.scenario) else {
            continue;
        };
        if b.total_frames == 0 {
            if r.total_frames > 0 {
                regressions.push(ScenarioRegression {
                    scenario: r.scenario.clone(),
                    baseline_p99_ms: 0.0,
                    current_p99_ms: r.p99_ms,
                    pct_delta: f64::INFINITY,
                });
            }
            continue;
        }
        if b.p99_ms <= 0.0 {
            continue;
        }
        let pct_delta = (r.p99_ms - b.p99_ms) / b.p99_ms;
        if pct_delta > threshold {
            regressions.push(ScenarioRegression {
                scenario: r.scenario.clone(),
                baseline_p99_ms: b.p99_ms,
                current_p99_ms: r.p99_ms,
                pct_delta,
            });
        }
    }
    regressions
}

pub fn incomplete_scenarios(results: &[BenchResults]) -> Vec<String> {
    results
        .iter()
        .filter(|result| !result.completed)
        .map(|result| result.scenario.clone())
        .collect()
}

pub fn invalid_baseline_scenarios(
    results: &[BenchResults],
    baseline: &[BenchResults],
) -> Vec<String> {
    results
        .iter()
        .filter(|result| {
            baseline
                .iter()
                .find(|entry| entry.scenario == result.scenario)
                .is_none_or(|entry| {
                    !entry.completed
                        || !entry.p99_ms.is_finite()
                        || (entry.total_frames > 0 && entry.p99_ms <= 0.0)
                })
        })
        .map(|result| result.scenario.clone())
        .collect()
}

pub fn baseline_environment_mismatches(
    current: &BenchEnvironment,
    baseline: &BenchEnvironment,
) -> Vec<String> {
    let mut mismatches = Vec::new();
    for (name, current, baseline) in [
        (
            "platform",
            current.platform.as_str(),
            baseline.platform.as_str(),
        ),
        (
            "terminal",
            current.terminal.as_str(),
            baseline.terminal.as_str(),
        ),
        (
            "build profile",
            current.build_profile.as_str(),
            baseline.build_profile.as_str(),
        ),
    ] {
        if current != baseline {
            mismatches.push(format!(
                "{name}: current {current:?}, baseline {baseline:?}"
            ));
        }
    }
    if current.rows != baseline.rows || current.cols != baseline.cols {
        mismatches.push(format!(
            "dimensions: current {}x{}, baseline {}x{}",
            current.cols, current.rows, baseline.cols, baseline.rows
        ));
    }
    mismatches
}

fn validate_report(report: &BenchReport) -> Result<()> {
    if report.schema_version != 1 {
        bail!(
            "unsupported benchmark schema version {}",
            report.schema_version
        );
    }
    if report.environment.machine.trim().is_empty()
        || report.environment.platform.trim().is_empty()
        || report.environment.terminal.trim().is_empty()
        || report.environment.build_profile.trim().is_empty()
        || report.environment.binary.trim().is_empty()
    {
        bail!("benchmark environment contains an empty value");
    }
    if report.environment.rows == 0 || report.environment.cols == 0 {
        bail!("benchmark terminal dimensions must be nonzero");
    }
    if report.scenario_count != report.results.len() {
        bail!(
            "scenario count is {}, but the report contains {} result(s)",
            report.scenario_count,
            report.results.len()
        );
    }
    if let Some(result) = report.results.iter().find(|result| !result.completed) {
        bail!("scenario {:?} did not complete", result.scenario);
    }
    if let Some(result) = report
        .results
        .iter()
        .find(|result| result.scenario.trim().is_empty())
    {
        bail!("benchmark report contains an empty scenario name: {result:?}");
    }
    let scenarios = report
        .results
        .iter()
        .map(|result| result.scenario.as_str())
        .collect::<BTreeSet<_>>();
    if scenarios.len() != report.results.len() {
        bail!("benchmark report contains duplicate scenario names");
    }
    let event_count = report
        .results
        .iter()
        .map(|result| result.total_frames)
        .sum::<u64>();
    if report.event_count != event_count {
        bail!(
            "event count is {}, but the results contain {} measured frame event(s)",
            report.event_count,
            event_count
        );
    }
    Ok(())
}

/// `pct` in `[0.0, 100.0]`. Input must be sorted ascending (debug-asserted).
pub fn percentile(sorted: &[f64], pct: f64) -> f64 {
    debug_assert!(
        (0.0..=100.0).contains(&pct),
        "percentile must be in [0.0, 100.0], got {pct}"
    );
    debug_assert!(
        sorted.windows(2).all(|w| w[0] <= w[1]),
        "input must be sorted"
    );
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (pct / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> BenchEnvironment {
        BenchEnvironment {
            machine: "test-host".to_owned(),
            platform: "linux-x86_64".to_owned(),
            terminal: "xterm-256color".to_owned(),
            rows: 50,
            cols: 120,
            build_profile: "release".to_owned(),
            binary: "/tmp/bot".to_owned(),
        }
    }

    #[test]
    fn empty_timings_returns_completed_zeroed_result() {
        let results = BenchResults::from_timings("empty", &[], Duration::from_secs(1));
        assert!(results.completed);
        assert_eq!(results.total_frames, 0);
        assert_eq!(results.avg_fps, 0.0);
        assert_eq!(results.p50_ms, 0.0);
        assert_eq!(results.p99_ms, 0.0);
        assert_eq!(results.max_ms, 0.0);
        assert_eq!(results.jank_count, 0);
        assert_eq!(results.chars_per_frame_avg, 0.0);
    }

    #[test]
    fn failed_results_fail_measurement_validation() {
        let missing = BenchResults::failed("missing");
        let measured = BenchResults::from_timings(
            "measured",
            &[FrameTiming {
                duration: Duration::from_millis(1),
                chars: 1,
            }],
            Duration::from_secs(1),
        );

        assert_eq!(incomplete_scenarios(&[missing, measured]), ["missing"]);
    }

    #[test]
    fn completed_idle_result_passes_measurement_validation() {
        let idle = BenchResults::from_timings("idle_cost", &[], Duration::from_secs(1));

        assert!(incomplete_scenarios(&[idle]).is_empty());
    }

    #[test]
    fn incomplete_or_unmeasured_baselines_fail_validation() {
        let results = [
            BenchResults::from_timings(
                "measured",
                &[FrameTiming {
                    duration: Duration::from_millis(1),
                    chars: 1,
                }],
                Duration::from_secs(1),
            ),
            BenchResults::from_timings(
                "missing",
                &[FrameTiming {
                    duration: Duration::from_millis(1),
                    chars: 1,
                }],
                Duration::from_secs(1),
            ),
            BenchResults::from_timings(
                "empty",
                &[FrameTiming {
                    duration: Duration::from_millis(1),
                    chars: 1,
                }],
                Duration::from_secs(1),
            ),
        ];
        let baseline = [results[0].clone(), BenchResults::failed("empty")];

        assert_eq!(
            invalid_baseline_scenarios(&results, &baseline),
            ["missing", "empty"]
        );
    }

    #[test]
    fn report_records_provenance_and_event_count() {
        let results = vec![BenchResults::from_timings(
            "measured",
            &[
                FrameTiming {
                    duration: Duration::from_millis(1),
                    chars: 1,
                },
                FrameTiming {
                    duration: Duration::from_millis(2),
                    chars: 1,
                },
            ],
            Duration::from_secs(1),
        )];
        let report = BenchReport::new(environment(), results);

        assert_eq!(report.schema_version, 1);
        assert_eq!(report.scenario_count, 1);
        assert_eq!(report.event_count, 2);
        assert_eq!(report.environment.machine, "test-host");
        assert!(validate_report(&report).is_ok());
    }

    #[test]
    fn baseline_round_trip_preserves_provenance() {
        let result = BenchResults::from_timings(
            "measured",
            &[FrameTiming {
                duration: Duration::from_millis(1),
                chars: 1,
            }],
            Duration::from_secs(1),
        );
        let report = BenchReport::new(environment(), vec![result]);
        let dir = tempfile::tempdir().expect("create temporary directory");
        let path = dir.path().join("baseline.json");

        write_baseline(&path, &report).expect("write baseline");
        let loaded = load_baseline(&path).expect("load baseline");

        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.environment, report.environment);
        assert_eq!(loaded.event_count, 1);
        assert_eq!(loaded.results.len(), 1);
    }

    #[test]
    fn failed_result_cannot_be_written_as_a_baseline() {
        let report = BenchReport::new(environment(), vec![BenchResults::failed("failed")]);
        let dir = tempfile::tempdir().expect("create temporary directory");
        let path = dir.path().join("baseline.json");

        let error = write_baseline(&path, &report).expect_err("reject failed baseline");

        assert!(format!("{error:#}").contains("did not complete"));
        assert!(!path.exists());
    }

    #[test]
    fn baseline_comparison_rejects_incompatible_context() {
        let current = environment();
        let mut baseline = current.clone();
        baseline.terminal = "screen-256color".to_owned();
        baseline.rows = 40;

        assert_eq!(
            baseline_environment_mismatches(&current, &baseline),
            [
                "terminal: current \"xterm-256color\", baseline \"screen-256color\"",
                "dimensions: current 120x50, baseline 120x40",
            ]
        );
    }

    #[test]
    fn idle_frames_regress_from_a_zero_frame_baseline() {
        let baseline = [BenchResults::from_timings(
            "idle_cost",
            &[],
            Duration::from_secs(1),
        )];
        let current = [BenchResults::from_timings(
            "idle_cost",
            &[FrameTiming {
                duration: Duration::from_millis(16),
                chars: 1,
            }],
            Duration::from_secs(1),
        )];

        let regressions = compare_baseline(&current, &baseline, 0.15);

        assert_eq!(regressions.len(), 1);
        assert_eq!(regressions[0].scenario, "idle_cost");
        assert!(regressions[0].pct_delta.is_infinite());
    }

    #[test]
    fn single_frame_returns_correct_values() {
        let timings = vec![FrameTiming {
            duration: Duration::from_millis(16),
            chars: 100,
        }];
        let results = BenchResults::from_timings("single", &timings, Duration::from_secs(1));
        assert_eq!(results.total_frames, 1);
        assert!((results.avg_fps - 1.0).abs() < 0.01);
        assert!((results.p50_ms - 16.0).abs() < 0.1);
        assert!((results.p99_ms - 16.0).abs() < 0.1);
        assert!((results.max_ms - 16.0).abs() < 0.1);
        assert_eq!(results.jank_count, 0);
        assert!((results.chars_per_frame_avg - 100.0).abs() < 0.01);
    }

    #[test]
    fn multiple_frames_statistics() {
        let timings: Vec<FrameTiming> = (0..100)
            .map(|i| FrameTiming {
                duration: Duration::from_millis(10 + i % 5),
                chars: 50,
            })
            .collect();
        let results = BenchResults::from_timings("multi", &timings, Duration::from_secs(2));
        assert_eq!(results.total_frames, 100);
        assert!((results.avg_fps - 50.0).abs() < 0.01);
        assert!(results.p50_ms >= 10.0 && results.p50_ms <= 14.0);
        assert!(results.p99_ms >= 10.0 && results.p99_ms <= 14.0);
        assert!((results.max_ms - 14.0).abs() < 0.1);
    }

    #[test]
    fn percentile_empty_returns_zero() {
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn percentile_single_element() {
        assert_eq!(percentile(&[42.0], 50.0), 42.0);
        assert_eq!(percentile(&[42.0], 0.0), 42.0);
        assert_eq!(percentile(&[42.0], 100.0), 42.0);
    }

    #[test]
    fn percentile_multiple_elements() {
        let sorted: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let p50 = percentile(&sorted, 50.0);
        assert!((p50 - 50.0).abs() < 1.1);
        let p99 = percentile(&sorted, 99.0);
        assert!((p99 - 99.0).abs() < 1.1);
    }

    #[test]
    fn jank_detection() {
        // Nine 10ms frames plus one 50ms frame, which exceeds 2x the median
        let mut timings: Vec<FrameTiming> = (0..9)
            .map(|_| FrameTiming {
                duration: Duration::from_millis(10),
                chars: 10,
            })
            .collect();
        timings.push(FrameTiming {
            duration: Duration::from_millis(50),
            chars: 10,
        });
        let results = BenchResults::from_timings("jank", &timings, Duration::from_secs(1));
        assert_eq!(results.jank_count, 1);
        assert!((results.jank_rate - 0.1).abs() < 0.01);
    }
}
