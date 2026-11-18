// SPDX-License-Identifier: Apache-2.0

mod config;
mod kube;
mod log;
mod metrics;
mod model;
mod nvml;
mod process;
mod value;

use config::Config;
use metrics::build_info::{BUILD_INFO, register_build_info};
use metrics::provider::GpuMetricsProvider;
use nvml::collector::MetricsCollector;
use prometheus::Registry;
use prometheus::process_collector::ProcessCollector;
use std::process::ExitCode;
use std::sync::Arc;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

fn main() -> ExitCode {
    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
    match runtime.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log!("run exporter: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Startup order and error handling follow Go: config, build info, kube store sync, then the HTTP server and the
/// NVML collector concurrently. The first one to stop cancels the other; its error is the result.
async fn run() -> anyhow::Result<()> {
    let cancel = CancellationToken::new();
    tokio::spawn(cancel_on_signal(cancel.clone()));

    let cfg = Config::load(&value::EnvProvider).map_err(anyhow::Error::msg)?;
    let registry = Registry::new();
    registry.register(Box::new(ProcessCollector::for_self()))?;
    register_build_info(&registry, &BUILD_INFO);
    let gmp = Arc::new(GpuMetricsProvider::new(&registry));
    let store = kube::store::Store::start(&cfg, gmp.clone(), cancel.clone()).await?;

    let collector = MetricsCollector::new(store, cfg.driver_poll_interval, gmp);
    let mut server = tokio::spawn(metrics::server::listen_and_serve(
        registry,
        cfg.exporter_endpoint.clone(),
        cfg.exporter_port,
        cancel.clone(),
    ));
    let collector_cancel = cancel.clone();
    let mut collector = tokio::task::spawn_blocking(move || collector.run(collector_cancel));
    let (first, other) = tokio::select! {
        result = &mut server => (result, collector),
        result = &mut collector => (result, server),
    };
    cancel.cancel();
    let other = other.await;
    first?.and(other?)
}

async fn cancel_on_signal(cancel: CancellationToken) {
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
    }
    cancel.cancel();
}
