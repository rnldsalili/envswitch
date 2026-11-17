//! One process-lifetime Job owns op children; Windows closes it even on process::exit.

use std::{
    ffi::c_void,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Child,
    sync::OnceLock,
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
    fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, size: u32)
    -> i32;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
}

#[repr(C)]
#[derive(Default)]
struct BasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct ExtendedLimitInformation {
    basic: BasicLimitInformation,
    io_counters: [u64; 6],
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

/// Create a non-inheritable Job whose last handle closing terminates its children.
pub(crate) fn create_job() -> Option<OwnedHandle> {
    // SAFETY: null attributes/name create a private, non-inheritable Job.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: CreateJobObjectW returned a new, valid handle owned by this function.
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    let limits = ExtendedLimitInformation {
        basic: BasicLimitInformation {
            limit_flags: 0x2000, // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: class 9 takes this repr(C) extended-limit structure and its exact size.
    (unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            9,
            (&limits as *const ExtendedLimitInformation).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } != 0)
        .then_some(job)
}

pub(crate) fn process_job() -> Option<&'static OwnedHandle> {
    static JOB: OnceLock<Option<OwnedHandle>> = OnceLock::new();
    JOB.get_or_init(create_job).as_ref()
}

pub(crate) fn assign_child(job: &OwnedHandle, child: &Child) {
    // SAFETY: both handles are live. Restrictive host Jobs may reject nesting;
    // keep running in the isolated console group if assignment is unavailable.
    let _ = unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) };
}
