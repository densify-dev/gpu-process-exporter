//! Fake `libnvidia-ml.so.1` for the parity harness.
//!
//! `FAKE_NVML_DIR` holds `scenario.json` (frames), `frame` (current frame index, written by the harness),
//! and receives `ack.<pid>` (frame served on `nvmlDeviceGetCount_v2`) and `trace.<pid>` (call trace).
//! A frame is snapshotted on `nvmlInit_v2` and on every `nvmlDeviceGetCount_v2`, so one collection cycle
//! always sees one frame.
#![allow(non_snake_case, clippy::missing_safety_doc)]

mod scenario;

use nvml_wrapper_sys::bindings::*;
use scenario::{Device, Frame, Scenario};
use std::ffi::{c_char, c_uint, c_ulonglong};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

const SUCCESS: nvmlReturn_t = nvmlReturn_enum_NVML_SUCCESS;
const INVALID_ARGUMENT: nvmlReturn_t = nvmlReturn_enum_NVML_ERROR_INVALID_ARGUMENT;
const NOT_FOUND: nvmlReturn_t = nvmlReturn_enum_NVML_ERROR_NOT_FOUND;
const INSUFFICIENT_SIZE: nvmlReturn_t = nvmlReturn_enum_NVML_ERROR_INSUFFICIENT_SIZE;
const NOT_SUPPORTED: nvmlReturn_t = nvmlReturn_enum_NVML_ERROR_NOT_SUPPORTED;
const VERSION_MISMATCH: nvmlReturn_t = nvmlReturn_enum_NVML_ERROR_ARGUMENT_VERSION_MISMATCH;

struct State {
    dir: PathBuf,
    scenario: Scenario,
    index: usize,
}

impl State {
    fn load() -> State {
        let dir =
            PathBuf::from(std::env::var_os("FAKE_NVML_DIR").expect("FAKE_NVML_DIR is not set"));
        let raw = fs::read(dir.join("scenario.json")).expect("read scenario.json");
        let scenario = serde_json::from_slice(&raw).expect("parse scenario.json");
        let mut state = State {
            dir,
            scenario,
            index: 0,
        };
        state.snapshot();
        state
    }

    fn snapshot(&mut self) {
        let raw = fs::read_to_string(self.dir.join("frame")).unwrap_or_default();
        let index = raw.trim().parse().unwrap_or(0);
        self.index = index.min(self.scenario.frames.len().saturating_sub(1));
    }

    fn frame(&self) -> &Frame {
        &self.scenario.frames[self.index]
    }

    fn device(&self, handle: nvmlDevice_t) -> Option<(usize, &Device)> {
        let index = (handle as usize).checked_sub(1)?;
        self.frame().devices.get(index).map(|d| (index, d))
    }

    /// Injected return code for `func`, optionally scoped to one device index.
    fn injected(&self, func: &str, device: Option<usize>) -> Option<nvmlReturn_t> {
        let returns = &self.frame().returns;
        device
            .and_then(|i| returns.get(&format!("{func}:{i}")))
            .or_else(|| returns.get(func))
            .copied()
    }

    fn write_file(&self, name: String, line: &str, append: bool) {
        let path = self.dir.join(name);
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(path)
            .expect("open fake NVML output");
        writeln!(file, "{line}").expect("write fake NVML output");
    }

    fn trace(&self, func: &str, device: Option<usize>, extra: &str, ret: nvmlReturn_t) {
        let dev = device.map(|d| format!(" dev={d}")).unwrap_or_default();
        let line = format!("frame={} {func}{dev}{extra} ret={ret}", self.index);
        self.write_file(format!("trace.{}", std::process::id()), &line, true);
    }
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(State::load()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

const fn struct_version<T>(version: u32) -> c_uint {
    size_of::<T>() as c_uint | (version << 24)
}

/// Runs a per-device call: resolves the handle, applies injected return codes, and traces the outcome.
fn with_device(
    func: &str,
    handle: nvmlDevice_t,
    extra: &str,
    body: impl FnOnce(&Device) -> nvmlReturn_t,
) -> nvmlReturn_t {
    let st = state();
    let Some((index, device)) = st.device(handle) else {
        st.trace(func, None, extra, INVALID_ARGUMENT);
        return INVALID_ARGUMENT;
    };
    let ret = st
        .injected(func, Some(index))
        .unwrap_or_else(|| body(device));
    if ret != INSUFFICIENT_SIZE {
        st.trace(func, Some(index), extra, ret);
    }
    ret
}

/// Implements the NVML sizing protocol: report `items.len()` through `count` and return
/// `INSUFFICIENT_SIZE` when `out` is null or too small, else copy the items.
unsafe fn fill<T: Copy>(items: &[T], count: *mut c_uint, out: *mut T) -> nvmlReturn_t {
    unsafe {
        let capacity = *count as usize;
        *count = items.len() as c_uint;
        if items.is_empty() {
            return SUCCESS;
        }
        if out.is_null() || capacity < items.len() {
            return INSUFFICIENT_SIZE;
        }
        std::ptr::copy_nonoverlapping(items.as_ptr(), out, items.len());
    }
    SUCCESS
}

unsafe fn write_str(value: &str, buf: *mut c_char, len: c_uint) -> nvmlReturn_t {
    if buf.is_null() || value.len() + 1 > len as usize {
        return INSUFFICIENT_SIZE;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(value.as_ptr() as *const c_char, buf, value.len());
        *buf.add(value.len()) = 0;
    }
    SUCCESS
}

fn global(func: &str, body: impl FnOnce(&mut State) -> nvmlReturn_t) -> nvmlReturn_t {
    let mut st = state();
    let ret = st.injected(func, None).unwrap_or_else(|| body(&mut st));
    st.trace(func, None, "", ret);
    ret
}

#[unsafe(no_mangle)]
pub extern "C" fn nvmlInit_v2() -> nvmlReturn_t {
    global("nvmlInit_v2", |st| {
        st.snapshot();
        SUCCESS
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn nvmlShutdown() -> nvmlReturn_t {
    global("nvmlShutdown", |_| SUCCESS)
}

#[unsafe(no_mangle)]
pub extern "C" fn nvmlErrorString(ret: nvmlReturn_t) -> *const c_char {
    match ret {
        SUCCESS => c"Success",
        INVALID_ARGUMENT => c"Invalid Argument",
        NOT_SUPPORTED => c"Not Supported",
        NOT_FOUND => c"Not Found",
        INSUFFICIENT_SIZE => c"Insufficient Size",
        VERSION_MISMATCH => c"Argument version mismatch",
        _ => c"Unknown Error",
    }
    .as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlSystemGetDriverVersion(buf: *mut c_char, len: c_uint) -> nvmlReturn_t {
    unsafe { write_str("999.99.99", buf, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetCount_v2(count: *mut c_uint) -> nvmlReturn_t {
    let mut st = state();
    st.snapshot();
    st.write_file(
        format!("ack.{}", std::process::id()),
        &st.index.to_string(),
        false,
    );
    let ret = st
        .injected("nvmlDeviceGetCount_v2", None)
        .unwrap_or_else(|| {
            unsafe { *count = st.frame().devices.len() as c_uint };
            SUCCESS
        });
    st.trace("nvmlDeviceGetCount_v2", None, "", ret);
    ret
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetHandleByIndex_v2(
    index: c_uint,
    handle: *mut nvmlDevice_t,
) -> nvmlReturn_t {
    let st = state();
    let index = index as usize;
    let ret = st
        .injected("nvmlDeviceGetHandleByIndex_v2", Some(index))
        .unwrap_or_else(|| {
            if index >= st.frame().devices.len() {
                return INVALID_ARGUMENT;
            }
            unsafe { *handle = (index + 1) as nvmlDevice_t };
            SUCCESS
        });
    st.trace("nvmlDeviceGetHandleByIndex_v2", Some(index), "", ret);
    ret
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetUUID(
    device: nvmlDevice_t,
    buf: *mut c_char,
    len: c_uint,
) -> nvmlReturn_t {
    with_device("nvmlDeviceGetUUID", device, "", |d| unsafe {
        write_str(&d.uuid, buf, len)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetName(
    device: nvmlDevice_t,
    buf: *mut c_char,
    len: c_uint,
) -> nvmlReturn_t {
    with_device("nvmlDeviceGetName", device, "", |d| unsafe {
        write_str(&d.name, buf, len)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetMemoryInfo(
    device: nvmlDevice_t,
    memory: *mut nvmlMemory_t,
) -> nvmlReturn_t {
    with_device("nvmlDeviceGetMemoryInfo", device, "", |d| {
        unsafe {
            *memory = nvmlMemory_t {
                total: d.memory_total,
                free: d.memory_total,
                used: 0,
            };
        }
        SUCCESS
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetMemoryInfo_v2(
    device: nvmlDevice_t,
    memory: *mut nvmlMemory_v2_t,
) -> nvmlReturn_t {
    with_device("nvmlDeviceGetMemoryInfo_v2", device, "", |d| unsafe {
        if (*memory).version != struct_version::<nvmlMemory_v2_t>(2) {
            return VERSION_MISMATCH;
        }
        (*memory).total = d.memory_total;
        (*memory).reserved = 0;
        (*memory).free = d.memory_total;
        (*memory).used = 0;
        SUCCESS
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetAccountingMode(
    device: nvmlDevice_t,
    mode: *mut nvmlEnableState_t,
) -> nvmlReturn_t {
    with_device("nvmlDeviceGetAccountingMode", device, "", |d| {
        unsafe { *mode = d.accounting as nvmlEnableState_t };
        SUCCESS
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetAccountingStats(
    device: nvmlDevice_t,
    pid: c_uint,
    stats: *mut nvmlAccountingStats_t,
) -> nvmlReturn_t {
    with_device(
        "nvmlDeviceGetAccountingStats",
        device,
        &format!(" pid={pid}"),
        |d| {
            let Some(s) = d.accounting_stats.get(&pid) else {
                return NOT_FOUND;
            };
            unsafe {
                *stats = nvmlAccountingStats_t {
                    gpuUtilization: s.gpu,
                    memoryUtilization: s.mem,
                    maxMemoryUsage: s.max_memory,
                    time: s.time,
                    startTime: s.start_time,
                    isRunning: s.is_running,
                    reserved: [0; 5],
                };
            }
            SUCCESS
        },
    )
}

unsafe fn running_processes(
    func: &str,
    device: nvmlDevice_t,
    count: *mut c_uint,
    out: *mut nvmlProcessInfo_t,
    pick: fn(&Device) -> &Vec<scenario::Process>,
) -> nvmlReturn_t {
    with_device(func, device, "", |d| {
        let items: Vec<nvmlProcessInfo_t> = pick(d)
            .iter()
            .map(|p| nvmlProcessInfo_t {
                pid: p.pid,
                usedGpuMemory: p.used_memory,
                gpuInstanceId: u32::MAX,
                computeInstanceId: u32::MAX,
            })
            .collect();
        unsafe { fill(&items, count, out) }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetComputeRunningProcesses_v3(
    device: nvmlDevice_t,
    count: *mut c_uint,
    out: *mut nvmlProcessInfo_t,
) -> nvmlReturn_t {
    unsafe {
        running_processes(
            "nvmlDeviceGetComputeRunningProcesses_v3",
            device,
            count,
            out,
            |d| &d.compute,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetGraphicsRunningProcesses_v3(
    device: nvmlDevice_t,
    count: *mut c_uint,
    out: *mut nvmlProcessInfo_t,
) -> nvmlReturn_t {
    unsafe {
        running_processes(
            "nvmlDeviceGetGraphicsRunningProcesses_v3",
            device,
            count,
            out,
            |d| &d.graphics,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetProcessUtilization(
    device: nvmlDevice_t,
    out: *mut nvmlProcessUtilizationSample_t,
    count: *mut c_uint,
    last_seen: c_ulonglong,
) -> nvmlReturn_t {
    let extra = format!(" last_seen={last_seen}");
    with_device("nvmlDeviceGetProcessUtilization", device, &extra, |d| {
        let items: Vec<nvmlProcessUtilizationSample_t> = d
            .samples
            .iter()
            .filter(|s| d.ignore_last_seen || s.ts > last_seen)
            .map(|s| nvmlProcessUtilizationSample_t {
                pid: s.pid,
                timeStamp: s.ts,
                smUtil: s.sm,
                memUtil: s.mem,
                encUtil: s.enc,
                decUtil: s.dec,
            })
            .collect();
        // The driver always answers a sizing call (null buffer) with INSUFFICIENT_SIZE.
        if out.is_null() {
            unsafe { *count = items.len() as c_uint };
            return INSUFFICIENT_SIZE;
        }
        unsafe { fill(&items, count, out) }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetProcessesUtilizationInfo(
    device: nvmlDevice_t,
    info: *mut nvmlProcessesUtilizationInfo_t,
) -> nvmlReturn_t {
    let last_seen = unsafe { (*info).lastSeenTimeStamp };
    let extra = format!(" last_seen={last_seen}");
    with_device(
        "nvmlDeviceGetProcessesUtilizationInfo",
        device,
        &extra,
        |d| unsafe {
            if (*info).version != struct_version::<nvmlProcessesUtilizationInfo_t>(1) {
                return VERSION_MISMATCH;
            }
            let items: Vec<nvmlProcessUtilizationInfo_v1_t> = d
                .extended_samples
                .iter()
                .filter(|s| s.ts > last_seen)
                .map(|s| nvmlProcessUtilizationInfo_v1_t {
                    timeStamp: s.ts,
                    pid: s.pid,
                    smUtil: s.sm,
                    memUtil: s.mem,
                    encUtil: s.enc,
                    decUtil: s.dec,
                    jpgUtil: s.jpg,
                    ofaUtil: s.ofa,
                })
                .collect();
            fill(
                &items,
                &mut (*info).processSamplesCount,
                (*info).procUtilArray,
            )
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetRunningProcessDetailList(
    device: nvmlDevice_t,
    list: *mut nvmlProcessDetailList_t,
) -> nvmlReturn_t {
    with_device(
        "nvmlDeviceGetRunningProcessDetailList",
        device,
        "",
        |d| unsafe {
            if (*list).version != struct_version::<nvmlProcessDetailList_t>(1) {
                return VERSION_MISMATCH;
            }
            if (*list).mode != 0 {
                return INVALID_ARGUMENT;
            }
            let items: Vec<nvmlProcessDetail_v1_t> = d
                .details
                .iter()
                .map(|p| nvmlProcessDetail_v1_t {
                    pid: p.pid,
                    usedGpuMemory: p.used_memory,
                    gpuInstanceId: u32::MAX,
                    computeInstanceId: u32::MAX,
                    usedGpuCcProtectedMemory: p.protected_memory,
                })
                .collect();
            fill(&items, &mut (*list).numProcArrayEntries, (*list).procArray)
        },
    )
}
