//! What the engine calls by its Unix name, as the Windows C runtime spells it. (`room` was written on Unix.)

use std::ffi::{c_char, c_int, c_void};

unsafe extern "C" {
    fn _stricmp(a: *const c_char, b: *const c_char) -> c_int;
    fn _strnicmp(a: *const c_char, b: *const c_char, n: usize) -> c_int;
    fn _strdup(s: *const c_char) -> *mut c_char;
    fn _mkdir(path: *const c_char) -> c_int;
    fn _errno() -> *mut c_int;
    fn __acrt_iob_func(n: u32) -> *mut c_void;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcasecmp(a: *const c_char, b: *const c_char) -> c_int {
    unsafe { _stricmp(a, b) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncasecmp(a: *const c_char, b: *const c_char, n: usize) -> c_int {
    unsafe { _strnicmp(a, b, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strdup(s: *const c_char) -> *mut c_char {
    unsafe { _strdup(s) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkdir(path: *const c_char, _mode: u32) -> c_int {
    unsafe { _mkdir(path) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __errno_location() -> *mut c_int {
    unsafe { _errno() }
}

/// In the runtime it's a function: `set_stdout` has to be called before the engine starts.
#[unsafe(no_mangle)]
pub static mut stdout: *mut c_void = std::ptr::null_mut();

pub fn set_stdout() {
    unsafe { stdout = __acrt_iob_func(1) };
}
