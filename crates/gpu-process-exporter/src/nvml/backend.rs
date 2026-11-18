// SPDX-License-Identifier: Apache-2.0

//! The NVML calls the collector makes, behind traits so tests can mock them. `NvmlBackend` calls the C library
//! directly through `nvml-wrapper-sys`, with the same buffer-sizing patterns as go-nvml.

use nvml_wrapper_sys::bindings::*;
use std::ffi::{CStr, c_char, c_uint};
use std::ptr;

/// An NVML return code other than success.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NvmlError(pub nvmlReturn_t);

const SUCCESS: nvmlReturn_t = nvmlReturn_enum_NVML_SUCCESS;
const INSUFFICIENT_SIZE: nvmlReturn_t = nvmlReturn_enum_NVML_ERROR_INSUFFICIENT_SIZE;
const FUNCTION_NOT_FOUND: nvmlReturn_t = nvmlReturn_enum_NVML_ERROR_FUNCTION_NOT_FOUND;
const LIB_PATH: &str = "libnvidia-ml.so.1";
const BUFFER_SIZE: usize = 96;

pub type NvmlResult<T> = Result<T, NvmlError>;

fn check(ret: nvmlReturn_t) -> NvmlResult<()> {
    if ret == SUCCESS {
        Ok(())
    } else {
        Err(NvmlError(ret))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub used_gpu_memory: u64,
}

/// A sample from either utilization API; the legacy one has no OFA or JPG values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UtilSample {
    pub pid: u32,
    pub timestamp: u64,
    pub sm: u32,
    pub mem: u32,
    pub enc: u32,
    pub dec: u32,
    pub ofa: u32,
    pub jpg: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProcessDetail {
    pub pid: u32,
    pub used_gpu_cc_protected_memory: u64,
}

/// Raw values, like go-nvml: no "not available" mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AccountingStats {
    pub gpu_utilization: u32,
    pub memory_utilization: u32,
    pub max_memory_usage: u64,
    pub time: u64,
    pub start_time: u64,
    pub is_running: u32,
}

pub trait GpuDevice {
    fn uuid(&self) -> NvmlResult<String>;
    fn name(&self) -> NvmlResult<String>;
    /// Total memory from `nvmlDeviceGetMemoryInfo_v2`.
    fn memory_total_v2(&self) -> NvmlResult<u64>;
    /// Total memory from `nvmlDeviceGetMemoryInfo`.
    fn memory_total(&self) -> NvmlResult<u64>;
    fn accounting_enabled(&self) -> NvmlResult<bool>;
    fn accounting_stats(&self, pid: u32) -> NvmlResult<AccountingStats>;
    fn compute_processes(&self) -> NvmlResult<Vec<ProcessInfo>>;
    fn graphics_processes(&self) -> NvmlResult<Vec<ProcessInfo>>;
    fn process_utilization(&self, last_seen: u64) -> NvmlResult<Vec<UtilSample>>;
    fn processes_utilization_info(&self, last_seen: u64) -> NvmlResult<Vec<UtilSample>>;
    fn process_details(&self) -> NvmlResult<Vec<ProcessDetail>>;
}

pub trait GpuBackend {
    fn device_count(&self) -> NvmlResult<u32>;
    fn device(&self, index: u32) -> NvmlResult<Box<dyn GpuDevice + '_>>;
    fn error_string(&self, err: NvmlError) -> String;
}

pub struct NvmlBackend {
    lib: NvmlLib,
}

/// Calls an optional symbol; a missing one returns `NVML_ERROR_FUNCTION_NOT_FOUND`.
macro_rules! call {
    ($lib:expr, $sym:ident($($arg:expr),*)) => {
        match $lib.$sym.as_ref() {
            Ok(f) => unsafe { f($($arg),*) },
            Err(_) => FUNCTION_NOT_FOUND,
        }
    };
}

impl NvmlBackend {
    /// Loads the library and calls `nvmlInit_v2`.
    pub fn init() -> anyhow::Result<NvmlBackend> {
        let lib = unsafe { NvmlLib::new(LIB_PATH) }
            .map_err(|e| anyhow::anyhow!("NVML init failed: load {LIB_PATH}: {e}"))?;
        let backend = NvmlBackend { lib };
        if let Err(e) = check(call!(backend.lib, nvmlInit_v2())) {
            anyhow::bail!("NVML init failed: {}", backend.error_string(e));
        }
        Ok(backend)
    }

    pub fn shutdown(self) -> anyhow::Result<()> {
        check(call!(self.lib, nvmlShutdown()))
            .map_err(|e| anyhow::anyhow!("NVML shutdown failed: {}", self.error_string(e)))
    }
}

impl GpuBackend for NvmlBackend {
    fn device_count(&self) -> NvmlResult<u32> {
        let mut count: c_uint = 0;
        check(call!(self.lib, nvmlDeviceGetCount_v2(&mut count)))?;
        Ok(count)
    }

    fn device(&self, index: u32) -> NvmlResult<Box<dyn GpuDevice + '_>> {
        let mut handle: nvmlDevice_t = ptr::null_mut();
        check(call!(
            self.lib,
            nvmlDeviceGetHandleByIndex_v2(index, &mut handle)
        ))?;
        Ok(Box::new(NvmlDevice {
            lib: &self.lib,
            handle,
        }))
    }

    fn error_string(&self, err: NvmlError) -> String {
        match self.lib.nvmlErrorString.as_ref() {
            Ok(f) => unsafe { CStr::from_ptr(f(err.0)) }
                .to_string_lossy()
                .into_owned(),
            Err(_) => format!("NVML error {}", err.0),
        }
    }
}

struct NvmlDevice<'a> {
    lib: &'a NvmlLib,
    handle: nvmlDevice_t,
}

const fn struct_version<T>(version: u32) -> c_uint {
    size_of::<T>() as c_uint | (version << 24)
}

impl NvmlDevice<'_> {
    fn string(&self, f: impl FnOnce(*mut c_char, c_uint) -> nvmlReturn_t) -> NvmlResult<String> {
        let mut buf = [0 as c_char; BUFFER_SIZE];
        check(f(buf.as_mut_ptr(), BUFFER_SIZE as c_uint))?;
        buf[BUFFER_SIZE - 1] = 0;
        Ok(unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned())
    }

    /// go-nvml's `_v3` pattern: start with one entry and double on `INSUFFICIENT_SIZE`.
    fn processes(
        &self,
        f: impl Fn(*mut c_uint, *mut nvmlProcessInfo_t) -> nvmlReturn_t,
    ) -> NvmlResult<Vec<ProcessInfo>> {
        let mut capacity: c_uint = 1;
        loop {
            let mut count = capacity;
            let mut infos =
                vec![unsafe { std::mem::zeroed::<nvmlProcessInfo_t>() }; capacity as usize];
            match f(&mut count, infos.as_mut_ptr()) {
                SUCCESS => {
                    infos.truncate(count as usize);
                    return Ok(infos
                        .iter()
                        .map(|p| ProcessInfo {
                            pid: p.pid,
                            used_gpu_memory: p.usedGpuMemory,
                        })
                        .collect());
                }
                INSUFFICIENT_SIZE => capacity = count.max(1) * 2,
                ret => return Err(NvmlError(ret)),
            }
        }
    }
}

impl GpuDevice for NvmlDevice<'_> {
    fn uuid(&self) -> NvmlResult<String> {
        self.string(|buf, len| call!(self.lib, nvmlDeviceGetUUID(self.handle, buf, len)))
    }

    fn name(&self) -> NvmlResult<String> {
        self.string(|buf, len| call!(self.lib, nvmlDeviceGetName(self.handle, buf, len)))
    }

    fn memory_total_v2(&self) -> NvmlResult<u64> {
        let mut memory: nvmlMemory_v2_t = unsafe { std::mem::zeroed() };
        memory.version = struct_version::<nvmlMemory_v2_t>(2);
        check(call!(
            self.lib,
            nvmlDeviceGetMemoryInfo_v2(self.handle, &mut memory)
        ))?;
        Ok(memory.total)
    }

    fn memory_total(&self) -> NvmlResult<u64> {
        let mut memory: nvmlMemory_t = unsafe { std::mem::zeroed() };
        check(call!(
            self.lib,
            nvmlDeviceGetMemoryInfo(self.handle, &mut memory)
        ))?;
        Ok(memory.total)
    }

    fn accounting_enabled(&self) -> NvmlResult<bool> {
        let mut mode: nvmlEnableState_t = 0;
        check(call!(
            self.lib,
            nvmlDeviceGetAccountingMode(self.handle, &mut mode)
        ))?;
        Ok(mode == nvmlEnableState_enum_NVML_FEATURE_ENABLED)
    }

    fn accounting_stats(&self, pid: u32) -> NvmlResult<AccountingStats> {
        let mut stats: nvmlAccountingStats_t = unsafe { std::mem::zeroed() };
        check(call!(
            self.lib,
            nvmlDeviceGetAccountingStats(self.handle, pid, &mut stats)
        ))?;
        Ok(AccountingStats {
            gpu_utilization: stats.gpuUtilization,
            memory_utilization: stats.memoryUtilization,
            max_memory_usage: stats.maxMemoryUsage,
            time: stats.time,
            start_time: stats.startTime,
            is_running: stats.isRunning,
        })
    }

    fn compute_processes(&self) -> NvmlResult<Vec<ProcessInfo>> {
        self.processes(|count, infos| {
            call!(
                self.lib,
                nvmlDeviceGetComputeRunningProcesses_v3(self.handle, count, infos)
            )
        })
    }

    fn graphics_processes(&self) -> NvmlResult<Vec<ProcessInfo>> {
        self.processes(|count, infos| {
            call!(
                self.lib,
                nvmlDeviceGetGraphicsRunningProcesses_v3(self.handle, count, infos)
            )
        })
    }

    /// go-nvml's pattern: size with a null buffer, which must answer `INSUFFICIENT_SIZE`, then fetch.
    fn process_utilization(&self, last_seen: u64) -> NvmlResult<Vec<UtilSample>> {
        let mut count: c_uint = 0;
        match call!(
            self.lib,
            nvmlDeviceGetProcessUtilization(self.handle, ptr::null_mut(), &mut count, last_seen)
        ) {
            INSUFFICIENT_SIZE => {}
            SUCCESS => return Ok(Vec::new()),
            ret => return Err(NvmlError(ret)),
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut samples =
            vec![unsafe { std::mem::zeroed::<nvmlProcessUtilizationSample_t>() }; count as usize];
        check(call!(
            self.lib,
            nvmlDeviceGetProcessUtilization(
                self.handle,
                samples.as_mut_ptr(),
                &mut count,
                last_seen
            )
        ))?;
        samples.truncate(count as usize);
        Ok(samples
            .iter()
            .map(|s| UtilSample {
                pid: s.pid,
                timestamp: s.timeStamp,
                sm: s.smUtil,
                mem: s.memUtil,
                enc: s.encUtil,
                dec: s.decUtil,
                ..Default::default()
            })
            .collect())
    }

    /// Sizes with a null array, then fetches. Go calls only once with a null array, so it never gets samples
    /// (see CONTRACT.md).
    fn processes_utilization_info(&self, last_seen: u64) -> NvmlResult<Vec<UtilSample>> {
        let mut info: nvmlProcessesUtilizationInfo_t = unsafe { std::mem::zeroed() };
        info.version = struct_version::<nvmlProcessesUtilizationInfo_t>(1);
        info.lastSeenTimeStamp = last_seen;
        match call!(
            self.lib,
            nvmlDeviceGetProcessesUtilizationInfo(self.handle, &mut info)
        ) {
            INSUFFICIENT_SIZE => {}
            SUCCESS => return Ok(Vec::new()),
            ret => return Err(NvmlError(ret)),
        }
        if info.processSamplesCount == 0 {
            return Ok(Vec::new());
        }
        let mut samples = vec![
            unsafe { std::mem::zeroed::<nvmlProcessUtilizationInfo_v1_t>() };
            info.processSamplesCount as usize
        ];
        info.procUtilArray = samples.as_mut_ptr();
        check(call!(
            self.lib,
            nvmlDeviceGetProcessesUtilizationInfo(self.handle, &mut info)
        ))?;
        samples.truncate(info.processSamplesCount as usize);
        Ok(samples
            .iter()
            .map(|s| UtilSample {
                pid: s.pid,
                timestamp: s.timeStamp,
                sm: s.smUtil,
                mem: s.memUtil,
                enc: s.encUtil,
                dec: s.decUtil,
                ofa: s.ofaUtil,
                jpg: s.jpgUtil,
            })
            .collect())
    }

    /// go-nvml's pattern: start with one entry and double on `INSUFFICIENT_SIZE`. Mode 0 lists compute processes.
    fn process_details(&self) -> NvmlResult<Vec<ProcessDetail>> {
        let mut list: nvmlProcessDetailList_t = unsafe { std::mem::zeroed() };
        list.version = struct_version::<nvmlProcessDetailList_t>(1);
        list.numProcArrayEntries = 1;
        loop {
            let mut details = vec![
                unsafe { std::mem::zeroed::<nvmlProcessDetail_v1_t>() };
                list.numProcArrayEntries as usize
            ];
            list.procArray = details.as_mut_ptr();
            match call!(
                self.lib,
                nvmlDeviceGetRunningProcessDetailList(self.handle, &mut list)
            ) {
                SUCCESS => {
                    details.truncate(list.numProcArrayEntries as usize);
                    return Ok(details
                        .iter()
                        .map(|d| ProcessDetail {
                            pid: d.pid,
                            used_gpu_cc_protected_memory: d.usedGpuCcProtectedMemory,
                        })
                        .collect());
                }
                INSUFFICIENT_SIZE => list.numProcArrayEntries = list.numProcArrayEntries.max(1) * 2,
                ret => return Err(NvmlError(ret)),
            }
        }
    }
}
