//! Scenario frames served by the fake. Every field defaults to empty so scenarios stay short.
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct Scenario {
    pub frames: Vec<Frame>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Frame {
    pub devices: Vec<Device>,
    /// Return codes keyed by `function` or `function:device_index`.
    pub returns: HashMap<String, u32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Device {
    pub uuid: String,
    pub name: String,
    pub memory_total: u64,
    pub accounting: bool,
    /// Serve legacy utilization samples at or below `lastSeen` too, like a driver that does not filter.
    pub ignore_last_seen: bool,
    pub compute: Vec<Process>,
    pub graphics: Vec<Process>,
    pub details: Vec<Process>,
    pub samples: Vec<Sample>,
    pub extended_samples: Vec<Sample>,
    pub accounting_stats: HashMap<u32, AccountingStats>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Process {
    pub pid: u32,
    pub used_memory: u64,
    pub protected_memory: u64,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Sample {
    pub pid: u32,
    pub ts: u64,
    pub sm: u32,
    pub mem: u32,
    pub enc: u32,
    pub dec: u32,
    pub jpg: u32,
    pub ofa: u32,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AccountingStats {
    pub gpu: u32,
    pub mem: u32,
    pub max_memory: u64,
    pub time: u64,
    pub start_time: u64,
    pub is_running: u32,
}
