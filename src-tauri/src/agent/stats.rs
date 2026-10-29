//! Host process measurements for `get_runtime_stats`.
//!
//! These are read from Windows, not inferred: a working set the kernel reports
//! for the emulator process is evidence, while "GPU acceleration makes it fast"
//! is not. Anything the platform refuses to answer stays `None`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessStats {
    pub working_set_mb: u64,
    /// Commit charge for the process, which is what the machine must back.
    pub private_mb: u64,
    /// Total user+kernel CPU time consumed so far.
    pub cpu_ms: u64,
}

#[cfg(windows)]
mod win {
    use super::ProcessStats;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        fn ticks(&self) -> u64 {
            ((self.high as u64) << 32) | self.low as u64
        }
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
        fn GetProcessTimes(
            process: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    pub fn process_stats(pid: u32) -> Option<ProcessStats> {
        if pid == 0 {
            return None;
        }
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle == 0 {
            return None;
        }
        let mut counters: ProcessMemoryCounters = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;

        let memory_ok = unsafe { K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) } != 0;

        let mut creation: FileTime = unsafe { std::mem::zeroed() };
        let mut exit: FileTime = unsafe { std::mem::zeroed() };
        let mut kernel: FileTime = unsafe { std::mem::zeroed() };
        let mut user: FileTime = unsafe { std::mem::zeroed() };
        let times_ok =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
                != 0;

        unsafe { CloseHandle(handle) };

        if !memory_ok && !times_ok {
            return None;
        }
        let mb = 1024 * 1024;
        Some(ProcessStats {
            working_set_mb: if memory_ok {
                (counters.working_set_size / mb) as u64
            } else {
                0
            },
            private_mb: if memory_ok {
                (counters.pagefile_usage / mb) as u64
            } else {
                0
            },
            // FILETIME counts 100 ns units; 10 000 of them are a millisecond.
            cpu_ms: if times_ok {
                (kernel.ticks() + user.ticks()) / 10_000
            } else {
                0
            },
        })
    }
}

#[cfg(not(windows))]
mod win {
    use super::ProcessStats;

    pub fn process_stats(_pid: u32) -> Option<ProcessStats> {
        None
    }
}

pub use win::process_stats;

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn the_current_process_reports_a_real_working_set() {
        let stats = process_stats(std::process::id()).expect("the process can read itself");
        assert!(
            stats.working_set_mb > 0,
            "a running process has resident memory"
        );
        assert!(stats.private_mb > 0);
    }

    #[test]
    fn a_process_that_does_not_exist_is_unavailable_not_zero() {
        assert!(process_stats(u32::MAX - 1).is_none());
        assert!(process_stats(0).is_none());
    }
}
