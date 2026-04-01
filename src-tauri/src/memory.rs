use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessMemoryStats {
    pub working_set_bytes: u64,
    pub private_bytes: Option<u64>,
    pub virtual_bytes: Option<u64>,
    pub peak_working_set_bytes: Option<u64>,
    pub source: String,
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn format_size_value(value: usize) -> String {
    if value >= 1024 * 1024 {
        format!("{:.2} MiB", value as f64 / (1024.0 * 1024.0))
    } else if value >= 1024 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}

pub fn memory_logging_enabled() -> bool {
    matches!(
        std::env::var("TAURSCRIBE_LOG_MEMORY").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("True")
    )
}

pub fn log_process_memory(label: &str) {
    let stats = process_memory_stats();
    print_process_memory_line(label, &stats, &[]);
}

fn print_process_memory_line(label: &str, stats: &ProcessMemoryStats, sizes: &[(&str, usize)]) {
    let private_mb = stats.private_bytes.map(mb);
    let virtual_mb = stats.virtual_bytes.map(mb);
    let peak_mb = stats.peak_working_set_bytes.map(mb);
    let extra = if sizes.is_empty() {
        String::new()
    } else {
        let joined = sizes
            .iter()
            .map(|(name, value)| format!("{name}={} ({})", value, format_size_value(*value)))
            .collect::<Vec<_>>()
            .join(" | ");
        format!(" | {joined}")
    };
    println!(
        "[MEMORY] {} | working_set={:.1} MB | private={} | virtual={} | peak_ws={} | source={}{}",
        label,
        mb(stats.working_set_bytes),
        private_mb
            .map(|v| format!("{v:.1} MB"))
            .unwrap_or_else(|| "n/a".to_string()),
        virtual_mb
            .map(|v| format!("{v:.1} MB"))
            .unwrap_or_else(|| "n/a".to_string()),
        peak_mb
            .map(|v| format!("{v:.1} MB"))
            .unwrap_or_else(|| "n/a".to_string()),
        stats.source,
        extra
    );
}

pub fn log_process_memory_with_sizes(label: &str, sizes: &[(&str, usize)]) {
    let stats = process_memory_stats();
    print_process_memory_line(label, &stats, sizes);
}

pub fn maybe_log_process_memory(label: &str) {
    if memory_logging_enabled() {
        log_process_memory(label);
    }
}

pub fn maybe_log_process_memory_with_sizes(label: &str, sizes: &[(&str, usize)]) {
    if memory_logging_enabled() {
        log_process_memory_with_sizes(label, sizes);
    }
}

pub fn trim_process_memory() {
    #[cfg(target_os = "windows")]
    {
        unsafe {
            #[link(name = "psapi")]
            extern "system" {
                fn EmptyWorkingSet(h_process: *mut std::ffi::c_void) -> i32;
            }
            #[link(name = "kernel32")]
            extern "system" {
                fn GetCurrentProcess() -> *mut std::ffi::c_void;
            }

            let handle = GetCurrentProcess();
            if !handle.is_null() {
                let result = EmptyWorkingSet(handle);
                if result == 0 {
                    eprintln!("[MEMORY] EmptyWorkingSet failed");
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        unsafe {
            #[link(name = "c")]
            extern "C" {
                fn malloc_trim(pad: usize) -> i32;
            }
            let _ = malloc_trim(0);
        }
    }
}

pub fn process_memory_stats() -> ProcessMemoryStats {
    #[cfg(target_os = "windows")]
    {
        if let Some(stats) = windows_process_memory_stats() {
            return stats;
        }
    }
    sysinfo_process_memory_stats()
}

fn sysinfo_process_memory_stats() -> ProcessMemoryStats {
    let mut system = sysinfo::System::new_all();
    system.refresh_all();

    let pid = match sysinfo::get_current_pid() {
        Ok(pid) => pid,
        Err(_) => {
            return ProcessMemoryStats {
                working_set_bytes: 0,
                private_bytes: None,
                virtual_bytes: None,
                peak_working_set_bytes: None,
                source: "unavailable".to_string(),
            }
        }
    };

    let Some(process) = system.process(pid) else {
        return ProcessMemoryStats {
            working_set_bytes: 0,
            private_bytes: None,
            virtual_bytes: None,
            peak_working_set_bytes: None,
            source: "unavailable".to_string(),
        };
    };

    ProcessMemoryStats {
        working_set_bytes: process.memory(),
        private_bytes: None,
        virtual_bytes: Some(process.virtual_memory()),
        peak_working_set_bytes: None,
        source: "sysinfo".to_string(),
    }
}

#[cfg(target_os = "windows")]
fn windows_process_memory_stats() -> Option<ProcessMemoryStats> {
    #[repr(C)]
    struct ProcessMemoryCountersEx {
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
        private_usage: usize,
    }

    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
        }
        #[link(name = "psapi")]
        extern "system" {
            fn GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                counters: *mut ProcessMemoryCountersEx,
                cb: u32,
            ) -> i32;
        }

        let handle = GetCurrentProcess();
        if handle.is_null() {
            return None;
        }

        let mut counters = ProcessMemoryCountersEx {
            cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
            private_usage: 0,
        };

        if GetProcessMemoryInfo(handle, &mut counters, counters.cb) == 0 {
            return None;
        }

        Some(ProcessMemoryStats {
            working_set_bytes: counters.working_set_size as u64,
            private_bytes: Some(counters.private_usage as u64),
            virtual_bytes: Some(counters.pagefile_usage as u64),
            peak_working_set_bytes: Some(counters.peak_working_set_size as u64),
            source: "windows_psapi".to_string(),
        })
    }
}
