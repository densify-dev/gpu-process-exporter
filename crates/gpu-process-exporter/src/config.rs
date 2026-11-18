// SPDX-License-Identifier: Apache-2.0

//! Runtime configuration from the environment.

use crate::value::{GoDuration, Provider, parse_value, parse_value_or_default};
use std::time::Duration;

pub const NODE_NAME_ENV_VAR: &str = "NODE_NAME";
pub const HOST_PROC_MOUNT_POINT_ENV_VAR: &str = "HOST_PROC_MOUNT_POINT";
pub const DEFAULT_HOST_PROC_MOUNT_POINT: &str = "/host/proc";
pub const DRIVER_POLL_INTERVAL_ENV_VAR: &str = "DRIVER_POLL_INTERVAL";
pub const DEFAULT_DRIVER_POLL_INTERVAL: GoDuration = GoDuration(1_000_000_000);
pub const SCRAPE_INTERVAL_ENV_VAR: &str = "SCRAPE_INTERVAL";
pub const DEFAULT_SCRAPE_INTERVAL: GoDuration = GoDuration(60_000_000_000);
pub const EXPORTER_ENDPOINT_ENV_VAR: &str = "EXPORTER_ENDPOINT";
pub const DEFAULT_EXPORTER_ENDPOINT: &str = "/metrics";
pub const EXPORTER_PORT_ENV_VAR: &str = "EXPORTER_PORT";
pub const DEFAULT_EXPORTER_PORT: u64 = 9494;
const MIN_ALLOWED_PORT: u64 = 1024;
const MAX_ALLOWED_PORT: u64 = 65535;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub node_name: String,
    pub host_proc_mount_point: String,
    pub driver_poll_interval: Duration,
    pub scrape_interval: Duration,
    pub exporter_endpoint: String,
    pub exporter_port: u16,
}

impl Config {
    pub fn load(provider: &dyn Provider) -> Result<Config, String> {
        let node_name: String = parse_value(provider, NODE_NAME_ENV_VAR)
            .map_err(|e| format!("parse {NODE_NAME_ENV_VAR}: {e}"))?;
        let host_proc_mount_point = parse_value_or_default(
            provider,
            HOST_PROC_MOUNT_POINT_ENV_VAR,
            DEFAULT_HOST_PROC_MOUNT_POINT.into(),
        );
        let driver_poll_interval = parse_value_or_default(
            provider,
            DRIVER_POLL_INTERVAL_ENV_VAR,
            DEFAULT_DRIVER_POLL_INTERVAL,
        );
        let scrape_interval =
            parse_value_or_default(provider, SCRAPE_INTERVAL_ENV_VAR, DEFAULT_SCRAPE_INTERVAL);
        let exporter_endpoint: String = parse_value_or_default(
            provider,
            EXPORTER_ENDPOINT_ENV_VAR,
            DEFAULT_EXPORTER_ENDPOINT.into(),
        );
        let exporter_port =
            parse_value_or_default(provider, EXPORTER_PORT_ENV_VAR, DEFAULT_EXPORTER_PORT);
        let invalid = |e: String| format!("invalid config: {e}");
        validate_intervals(driver_poll_interval, scrape_interval).map_err(invalid)?;
        validate_exporter_port(exporter_port).map_err(invalid)?;
        validate_exporter_endpoint(&exporter_endpoint).map_err(invalid)?;
        Ok(Config {
            node_name,
            host_proc_mount_point,
            driver_poll_interval: Duration::from_nanos(driver_poll_interval.0 as u64),
            scrape_interval: Duration::from_nanos(scrape_interval.0 as u64),
            exporter_endpoint,
            exporter_port: exporter_port as u16,
        })
    }
}

fn validate_intervals(
    driver_poll_interval: GoDuration,
    scrape_interval: GoDuration,
) -> Result<(), String> {
    if driver_poll_interval.0 <= 0 {
        return Err(format!(
            "{DRIVER_POLL_INTERVAL_ENV_VAR} must be > 0, got {}ns",
            driver_poll_interval.0
        ));
    }
    if scrape_interval.0 <= 0 {
        return Err(format!(
            "{SCRAPE_INTERVAL_ENV_VAR} must be > 0, got {}ns",
            scrape_interval.0
        ));
    }
    if scrape_interval.0 % driver_poll_interval.0 != 0 {
        return Err(format!(
            "{SCRAPE_INTERVAL_ENV_VAR} ({}ns) must be an exact multiple of {DRIVER_POLL_INTERVAL_ENV_VAR} ({}ns)",
            scrape_interval.0, driver_poll_interval.0
        ));
    }
    Ok(())
}

fn port_range() -> String {
    format!("between {MIN_ALLOWED_PORT} and {MAX_ALLOWED_PORT}")
}

fn validate_exporter_port(exporter_port: u64) -> Result<(), String> {
    if !(MIN_ALLOWED_PORT..=MAX_ALLOWED_PORT).contains(&exporter_port) {
        return Err(format!(
            "{EXPORTER_PORT_ENV_VAR} must be {}, got {exporter_port}",
            port_range()
        ));
    }
    Ok(())
}

/// Accepted deviation from Go: the endpoint is an exact path, so it must start with `/`.
fn validate_exporter_endpoint(endpoint: &str) -> Result<(), String> {
    if !endpoint.starts_with('/') {
        return Err(format!(
            "{EXPORTER_ENDPOINT_ENV_VAR} must start with '/', got {endpoint:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::MapProvider;
    use std::collections::BTreeMap;

    const SECOND: i64 = 1_000_000_000;
    const NODE_A: &str = "node-a";

    #[test]
    fn validate_intervals() {
        let cases = [
            (SECOND, 60 * SECOND, None),
            (0, 60 * SECOND, Some(DRIVER_POLL_INTERVAL_ENV_VAR)),
            (-SECOND, 60 * SECOND, Some(DRIVER_POLL_INTERVAL_ENV_VAR)),
            (SECOND, 0, Some(SCRAPE_INTERVAL_ENV_VAR)),
            (SECOND, -SECOND, Some(SCRAPE_INTERVAL_ENV_VAR)),
            (4 * SECOND, 10 * SECOND, Some("exact multiple")),
        ];
        for (poll, scrape, want) in cases {
            let got = super::validate_intervals(GoDuration(poll), GoDuration(scrape));
            match want {
                None => assert_eq!(got, Ok(()), "{poll}/{scrape}"),
                Some(want) => assert!(got.unwrap_err().contains(want), "{poll}/{scrape}"),
            }
        }
    }

    #[test]
    fn validate_exporter_port() {
        let range = port_range();
        let cases = [
            (8080, None),
            (0, Some(&range)),
            (MIN_ALLOWED_PORT - 1, Some(&range)),
            (MIN_ALLOWED_PORT, None),
            (MAX_ALLOWED_PORT, None),
            (MAX_ALLOWED_PORT + 1, Some(&range)),
        ];
        for (port, want) in cases {
            let got = super::validate_exporter_port(port);
            match want {
                None => assert_eq!(got, Ok(()), "{port}"),
                Some(want) => assert!(got.unwrap_err().contains(want.as_str()), "{port}"),
            }
        }
    }

    fn load(values: &[(&str, &str)]) -> Result<Config, String> {
        let m: BTreeMap<String, String> = values
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::load(&MapProvider(Some(&m)))
    }

    #[test]
    fn load_config_returns_errors() {
        let range = port_range();
        let cases: [(&[(&str, &str)], &str); 4] = [
            (&[], NODE_NAME_ENV_VAR),
            (
                &[
                    (NODE_NAME_ENV_VAR, NODE_A),
                    (DRIVER_POLL_INTERVAL_ENV_VAR, "4s"),
                    (SCRAPE_INTERVAL_ENV_VAR, "10s"),
                ],
                "exact multiple",
            ),
            (
                &[
                    (NODE_NAME_ENV_VAR, NODE_A),
                    (EXPORTER_PORT_ENV_VAR, "65536"),
                ],
                &range,
            ),
            (
                &[
                    (NODE_NAME_ENV_VAR, NODE_A),
                    (EXPORTER_ENDPOINT_ENV_VAR, "metrics"),
                ],
                "start with",
            ),
        ];
        for (values, want) in cases {
            assert!(load(values).unwrap_err().contains(want), "{values:?}");
        }
    }

    #[test]
    fn load_config_success() {
        let cfg = load(&[
            (NODE_NAME_ENV_VAR, NODE_A),
            (HOST_PROC_MOUNT_POINT_ENV_VAR, "/proc"),
            (DRIVER_POLL_INTERVAL_ENV_VAR, "2s"),
            (SCRAPE_INTERVAL_ENV_VAR, "10s"),
            (EXPORTER_ENDPOINT_ENV_VAR, "/custom-metrics"),
            (EXPORTER_PORT_ENV_VAR, "9443"),
        ])
        .unwrap();
        assert_eq!(
            cfg,
            Config {
                node_name: NODE_A.into(),
                host_proc_mount_point: "/proc".into(),
                driver_poll_interval: Duration::from_secs(2),
                scrape_interval: Duration::from_secs(10),
                exporter_endpoint: "/custom-metrics".into(),
                exporter_port: 9443,
            }
        );
    }

    #[test]
    fn load_config_defaults() {
        let cfg = load(&[(NODE_NAME_ENV_VAR, NODE_A), (EXPORTER_PORT_ENV_VAR, "abc")]).unwrap();
        assert_eq!(cfg.exporter_port, DEFAULT_EXPORTER_PORT as u16);
        assert_eq!(
            cfg.driver_poll_interval,
            Duration::from_nanos(DEFAULT_DRIVER_POLL_INTERVAL.0 as u64)
        );
        assert_eq!(cfg.host_proc_mount_point, DEFAULT_HOST_PROC_MOUNT_POINT);
    }
}
