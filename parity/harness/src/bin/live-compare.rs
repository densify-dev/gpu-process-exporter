//! Live parity check: scrapes a reference and a candidate exporter every interval and checks the L3 criteria
//! (plan section 1.2): equal series sets, equal gauges except one-scrape sampling skew, and counter increases
//! within 1% per window. Scrape errors (restarts) restart the warm-up and the counter window.
use anyhow::Result;
use clap::Parser;
use parity_harness::compare::{Diff, diff};
use parity_harness::expo::{Exposition, Labels};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

type Key = (String, Labels);

#[derive(Parser)]
struct Args {
    /// Reference metrics URL.
    #[arg(long)]
    reference: String,
    /// Candidate metrics URL.
    #[arg(long)]
    candidate: String,
    #[arg(long, default_value_t = 20)]
    interval_secs: u64,
    #[arg(long, default_value_t = 60)]
    duration_mins: u64,
    /// Counter comparison window.
    #[arg(long, default_value_t = 5)]
    window_mins: u64,
    /// Scrapes ignored after a start or a scrape error.
    #[arg(long, default_value_t = 2)]
    warmup: usize,
    /// Allowed relative difference of counter increases per window.
    #[arg(long, default_value_t = 0.01)]
    counter_tolerance: f64,
    /// The exporters come from different builds: skip build info label values.
    #[arg(long)]
    different_builds: bool,
}

#[derive(Default)]
struct Totals {
    scrapes: usize,
    compared: usize,
    errors: usize,
    /// Scrapes with a structural difference, and those whose difference persisted from the previous scrape.
    structure: usize,
    structure_persistent: usize,
    gauge: usize,
    gauge_persistent: usize,
    windows: usize,
    windows_failed: usize,
}

struct Window {
    reference: BTreeMap<Key, f64>,
    candidate: BTreeMap<Key, f64>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let scrapes = (args.duration_mins * 60 / args.interval_secs) as usize;
    let window_scrapes = (args.window_mins * 60 / args.interval_secs) as usize;
    let mut ticker = tokio::time::interval(Duration::from_secs(args.interval_secs));
    let mut totals = Totals::default();
    let mut warmup = args.warmup;
    let (mut prev_structure, mut prev_gauges) = (BTreeSet::<String>::new(), BTreeSet::<Key>::new());
    let mut window: Option<(usize, Window)> = None;

    for n in 0..scrapes {
        ticker.tick().await;
        totals.scrapes += 1;
        let ts = timestamp();
        let (r, c) = tokio::join!(
            scrape(&client, &args.reference),
            scrape(&client, &args.candidate)
        );
        let (r, c) = match (r, c) {
            (Ok(r), Ok(c)) => (r, c),
            (r, c) => {
                totals.errors += 1;
                warmup = args.warmup;
                window = None;
                let err = r.err().or(c.err()).unwrap();
                println!("{ts} scrape={n} error: {err:#}");
                continue;
            }
        };
        if warmup > 0 {
            warmup -= 1;
            println!("{ts} scrape={n} warm-up");
            continue;
        }
        totals.compared += 1;
        let (mut structure, mut gauges) = (BTreeSet::new(), BTreeMap::new());
        for d in diff(&r, &c, args.different_builds) {
            match d {
                Diff::Structure(m) => {
                    structure.insert(m);
                }
                Diff::Value {
                    key,
                    counter: false,
                    message,
                } => {
                    gauges.insert(key, message);
                }
                Diff::Value { counter: true, .. } => {}
            }
        }
        let persistent_structure: Vec<_> = structure.intersection(&prev_structure).collect();
        let persistent_gauges: Vec<_> =
            gauges.keys().filter(|k| prev_gauges.contains(*k)).collect();
        totals.structure += usize::from(!structure.is_empty());
        totals.structure_persistent += usize::from(!persistent_structure.is_empty());
        totals.gauge += usize::from(!gauges.is_empty());
        totals.gauge_persistent += usize::from(!persistent_gauges.is_empty());
        println!(
            "{ts} scrape={n} series={} structure_diffs={} gauge_diffs={} persistent={}",
            r.families.values().map(|f| f.samples.len()).sum::<usize>(),
            structure.len(),
            gauges.len(),
            persistent_structure.len() + persistent_gauges.len()
        );
        for m in structure.iter().chain(gauges.values()) {
            println!("  {m}");
        }

        let snapshot = Window {
            reference: counters(&r),
            candidate: counters(&c),
        };
        match &window {
            Some((start, base)) if n - start >= window_scrapes => {
                totals.windows += 1;
                let failed = compare_window(base, &snapshot, args.counter_tolerance);
                totals.windows_failed += usize::from(!failed.is_empty());
                println!(
                    "{ts} window scrapes={start}..{n} counter_failures={}",
                    failed.len()
                );
                for m in failed {
                    println!("  {m}");
                }
                window = Some((n, snapshot));
            }
            Some(_) => {}
            None => window = Some((n, snapshot)),
        }
        prev_structure = structure;
        prev_gauges = gauges.into_keys().collect();
    }

    let t = &totals;
    println!(
        "summary scrapes={} compared={} errors={} structure_diff_scrapes={} structure_persistent={} \
         gauge_diff_scrapes={} gauge_persistent={} counter_windows={} counter_windows_failed={}",
        t.scrapes,
        t.compared,
        t.errors,
        t.structure,
        t.structure_persistent,
        t.gauge,
        t.gauge_persistent,
        t.windows,
        t.windows_failed
    );
    let pass = t.compared > 0
        && t.structure_persistent == 0
        && t.gauge_persistent == 0
        && t.windows_failed == 0;
    println!("result {}", if pass { "PASS" } else { "FAIL" });
    std::process::exit(i32::from(!pass));
}

async fn scrape(client: &reqwest::Client, url: &str) -> Result<Exposition> {
    let text = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Exposition::parse(&text)
}

/// Exporter counters (`kubex_*`), keyed by series.
fn counters(expo: &Exposition) -> BTreeMap<Key, f64> {
    expo.families
        .iter()
        .filter(|(name, f)| name.starts_with("kubex_") && f.kind.as_deref() == Some("counter"))
        .flat_map(|(_, f)| f.samples.iter().map(|(k, v)| (k.clone(), *v)))
        .collect()
}

/// Compares counter increases over a window, for series present at both ends in both exporters.
fn compare_window(base: &Window, end: &Window, tolerance: f64) -> Vec<String> {
    let mut failed = Vec::new();
    for (key, r_end) in &end.reference {
        let (Some(r0), Some(c0), Some(c_end)) = (
            base.reference.get(key),
            base.candidate.get(key),
            end.candidate.get(key),
        ) else {
            continue;
        };
        let (dr, dc) = (r_end - r0, c_end - c0);
        if (dr - dc).abs() > tolerance * dr.abs().max(dc.abs()) {
            failed.push(format!(
                "{}{:?}: reference +{dr} candidate +{dc}",
                key.0, key.1
            ));
        }
    }
    failed
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("t={secs}")
}
