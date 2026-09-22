use std::env;

use ort::environment::GlobalThreadPoolOptions;

fn parse_usize_env(key: &str) -> Option<usize> {
    env::var(key).ok()?.trim().parse::<usize>().ok()
}

/// Commit a single low-RAM ORT environment before any session is created.
/// This lets all ORT sessions share one small global thread pool instead of
/// each session creating its own worker pool.
pub fn initialize_low_ram_ort_environment() -> Result<bool, String> {
    let intra_threads = parse_usize_env("TAURSCRIBE_ORT_INTRA_THREADS").unwrap_or(1);
    let inter_threads = parse_usize_env("TAURSCRIBE_ORT_INTER_THREADS").unwrap_or(1);

    let thread_pool = GlobalThreadPoolOptions::default()
        .with_intra_threads(intra_threads)
        .map_err(|e| format!("[ort-env] Set intra-op threads: {e}"))?
        .with_inter_threads(inter_threads)
        .map_err(|e| format!("[ort-env] Set inter-op threads: {e}"))?
        .with_spin_control(false)
        .map_err(|e| format!("[ort-env] Disable thread spinning: {e}"))?;

    Ok(ort::init().with_global_thread_pool(thread_pool).commit())
}
