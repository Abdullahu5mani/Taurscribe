use std::env;

use ort::environment::GlobalThreadPoolOptions;
use ort::execution_providers::cuda::ConvAlgorithmSearch;
use ort::execution_providers::{ArenaExtendStrategy, CUDAExecutionProvider};
use ort::session::builder::SessionBuilder;
use ort::{ortsys, AsPointer};

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

/// Build a CUDA EP configured for lower peak-memory growth.
pub fn build_low_ram_cuda_execution_provider() -> CUDAExecutionProvider {
    let mut ep = CUDAExecutionProvider::default()
        .with_arena_extend_strategy(ArenaExtendStrategy::SameAsRequested)
        .with_conv_algorithm_search(ConvAlgorithmSearch::Heuristic)
        .with_conv_max_workspace(false);

    if let Some(limit_mb) = parse_usize_env("TAURSCRIBE_ORT_CUDA_MEM_LIMIT_MB") {
        ep = ep.with_memory_limit(limit_mb.saturating_mul(1024 * 1024));
    }

    ep
}

/// Apply memory-conservative ONNX Runtime session settings for large ASR models.
pub fn configure_low_ram_session_builder(
    mut builder: SessionBuilder,
    label: &str,
) -> Result<SessionBuilder, String> {
    builder = builder
        .with_intra_threads(1)
        .map_err(|e| format!("[{label}] Set intra-op threads: {e}"))?
        .with_inter_threads(1)
        .map_err(|e| format!("[{label}] Set inter-op threads: {e}"))?
        .with_parallel_execution(false)
        .map_err(|e| format!("[{label}] Set parallel execution: {e}"))?
        .with_memory_pattern(false)
        .map_err(|e| format!("[{label}] Disable memory pattern: {e}"))?
        .with_prepacking(false)
        .map_err(|e| format!("[{label}] Disable prepacking: {e}"))?
        .with_inter_op_spinning(false)
        .map_err(|e| format!("[{label}] Disable inter-op spinning: {e}"))?
        .with_intra_op_spinning(false)
        .map_err(|e| format!("[{label}] Disable intra-op spinning: {e}"))?;

    (|| -> ort::Result<()> {
        ortsys![unsafe DisableCpuMemArena(builder.ptr_mut())?];
        Ok(())
    })()
    .map_err(|e| format!("[{label}] Disable CPU mem arena: {e}"))?;

    Ok(builder)
}
