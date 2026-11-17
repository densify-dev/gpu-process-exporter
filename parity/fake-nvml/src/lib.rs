#![allow(non_snake_case, clippy::missing_safety_doc)]
use nvml_wrapper_sys::bindings::*;
use std::ffi::{c_char, c_uint};

#[unsafe(no_mangle)]
pub extern "C" fn nvmlInit_v2() -> nvmlReturn_t { 0 }
#[unsafe(no_mangle)]
pub extern "C" fn nvmlShutdown() -> nvmlReturn_t { 0 }
#[unsafe(no_mangle)]
pub extern "C" fn nvmlErrorString(_r: nvmlReturn_t) -> *const c_char { c"fake".as_ptr() }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetCount_v2(c: *mut c_uint) -> nvmlReturn_t { unsafe { *c = 1 }; 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetHandleByIndex_v2(i: c_uint, d: *mut nvmlDevice_t) -> nvmlReturn_t {
    unsafe { *d = (i as usize + 1) as nvmlDevice_t }; 0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvmlDeviceGetUUID(_d: nvmlDevice_t, buf: *mut c_char, len: c_uint) -> nvmlReturn_t {
    let s = b"GPU-fake-0\0";
    if (len as usize) < s.len() { return 7 }
    unsafe { std::ptr::copy_nonoverlapping(s.as_ptr() as *const c_char, buf, s.len()) }; 0
}
