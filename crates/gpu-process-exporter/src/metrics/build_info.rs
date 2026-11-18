// SPDX-License-Identifier: Apache-2.0

//! The `kubex_gpu_process_exporter_build_info` gauge.

use crate::log;
use prometheus::{Gauge, Opts, Registry};

pub const BUILD_INFO_NAME: &str = "kubex_gpu_process_exporter_build_info";
const PROGRAM: &str = "kubex_gpu_process_exporter";

pub struct BuildInfo {
    pub version: &'static str,
    pub revision: &'static str,
    pub branch: &'static str,
    pub rust_version: &'static str,
}

pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: env!("BUILD_VERSION"),
    revision: env!("BUILD_REVISION"),
    branch: env!("BUILD_BRANCH"),
    rust_version: env!("BUILD_RUST_VERSION"),
};

pub fn register_build_info(registry: &Registry, info: &BuildInfo) {
    log!(
        "gpu-process-exporter build info: (version={}, branch={}, revision={}), (rust={}, platform={}/{})",
        info.version,
        info.branch,
        info.revision,
        info.rust_version,
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let opts = Opts::new(
        BUILD_INFO_NAME,
        format!(
            "A metric with a constant '1' value labeled by version, revision, branch, rustversion from which \
             {PROGRAM} was built, and the os and arch for the build."
        ),
    )
    .const_label("version", info.version)
    .const_label("revision", info.revision)
    .const_label("branch", info.branch)
    .const_label("rustversion", info.rust_version)
    .const_label("os", std::env::consts::OS)
    .const_label("arch", std::env::consts::ARCH);
    let gauge = Gauge::with_opts(opts).unwrap();
    gauge.set(1.0);
    registry.register(Box::new(gauge)).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn build_info() {
        let plain = BuildInfo {
            version: "",
            revision: "",
            branch: "",
            rust_version: BUILD_INFO.rust_version,
        };
        let release = BuildInfo {
            version: "v1.2.3",
            revision: "0123456789abcdef0123456789abcdef01234567",
            branch: "main",
            rust_version: BUILD_INFO.rust_version,
        };
        for info in [plain, release] {
            let registry = Registry::new();
            register_build_info(&registry, &info);
            let families = registry.gather();
            let family = families
                .iter()
                .find(|f| f.name() == BUILD_INFO_NAME)
                .expect("build info family");
            let metrics = family.get_metric();
            assert_eq!(metrics.len(), 1);
            assert_eq!(metrics[0].get_gauge().value(), 1.0);
            let labels: BTreeMap<&str, &str> = metrics[0]
                .get_label()
                .iter()
                .map(|l| (l.name(), l.value()))
                .collect();
            let want = BTreeMap::from([
                ("version", info.version),
                ("revision", info.revision),
                ("branch", info.branch),
                ("rustversion", info.rust_version),
                ("os", std::env::consts::OS),
                ("arch", std::env::consts::ARCH),
            ]);
            assert_eq!(labels, want);
        }
    }
}
