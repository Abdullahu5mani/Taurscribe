use std::path::{Path, PathBuf};
use std::time::Instant;

use ort::execution_providers::cuda::ConvAlgorithmSearch;
use ort::execution_providers::{ArenaExtendStrategy, CUDAExecutionProvider};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

fn model_dir() -> PathBuf {
    std::env::var_os("TAURSCRIBE_GRANITE_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .expect("LOCALAPPDATA is required on Windows");
            base.join("Taurscribe")
                .join("models")
                .join("granite-speech-4.1-2b-nar-cuda")
        })
}

fn cuda_provider() -> CUDAExecutionProvider {
    CUDAExecutionProvider::default()
        .with_arena_extend_strategy(ArenaExtendStrategy::SameAsRequested)
        .with_conv_algorithm_search(ConvAlgorithmSearch::Heuristic)
        .with_conv_max_workspace(false)
}

fn make_cuda_session(path: &Path, low_ram: bool) -> Result<Session, String> {
    let builder = Session::builder()
        .map_err(|e| format!("ORT builder: {e}"))?
        .with_execution_providers([cuda_provider().build().error_on_failure()])
        .map_err(|e| format!("CUDA EP: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| format!("CUDA opt level: {e}"))?;

    let mut builder = if low_ram {
        builder
            .with_intra_threads(1)
            .map_err(|e| format!("CUDA intra threads: {e}"))?
            .with_inter_threads(1)
            .map_err(|e| format!("CUDA inter threads: {e}"))?
            .with_parallel_execution(false)
            .map_err(|e| format!("CUDA parallel execution: {e}"))?
            .with_memory_pattern(false)
            .map_err(|e| format!("CUDA memory pattern: {e}"))?
            .with_prepacking(false)
            .map_err(|e| format!("CUDA prepacking: {e}"))?
            .with_inter_op_spinning(false)
            .map_err(|e| format!("CUDA inter spinning: {e}"))?
            .with_intra_op_spinning(false)
            .map_err(|e| format!("CUDA intra spinning: {e}"))?
    } else {
        builder
            .with_parallel_execution(true)
            .map_err(|e| format!("CUDA parallel execution: {e}"))?
            .with_memory_pattern(true)
            .map_err(|e| format!("CUDA memory pattern: {e}"))?
            .with_prepacking(true)
            .map_err(|e| format!("CUDA prepacking: {e}"))?
    };

    builder
        .commit_from_file(path)
        .map_err(|e| format!("CUDA session load {}: {e}", path.display()))
}

fn make_trt_session(
    path: &Path,
    cache_dir: &Path,
    fp16: bool,
    cuda_graph: bool,
) -> Result<Session, String> {
    let cache = cache_dir.to_string_lossy().to_string();
    let trt = ort::ep::TensorRT::default()
        .with_device_id(0)
        .with_fp16(fp16)
        .with_engine_cache(true)
        .with_engine_cache_path(cache.clone())
        .with_timing_cache(true)
        .with_timing_cache_path(cache)
        .with_builder_optimization_level(3)
        .with_max_workspace_size(4 * 1024 * 1024 * 1024usize)
        .with_cuda_graph(cuda_graph)
        .build()
        .error_on_failure();

    let cuda = cuda_provider().build().error_on_failure();

    let mut builder = Session::builder()
        .map_err(|e| format!("ORT builder: {e}"))?
        .with_execution_providers([trt, cuda])
        .map_err(|e| format!("TensorRT EP: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| format!("TRT opt level: {e}"))?
        .with_parallel_execution(true)
        .map_err(|e| format!("TRT parallel execution: {e}"))?
        .with_memory_pattern(true)
        .map_err(|e| format!("TRT memory pattern: {e}"))?
        .with_prepacking(true)
        .map_err(|e| format!("TRT prepacking: {e}"))?;

    builder
        .commit_from_file(path)
        .map_err(|e| format!("TRT session load {}: {e}", path.display()))
}

fn main() -> Result<(), String> {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "cuda-lowram".to_string());
    let dir = model_dir();
    let cache_dir = std::env::temp_dir().join("taurscribe-granite-trt-cache");
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create cache dir: {e}"))?;

    match mode.as_str() {
        "cuda-lowram" => {
            let _ = ort::init().commit();
        }
        _ => {
            let _ = ort::init().commit();
        }
    }

    let graphs = [
        ("encoder", dir.join("encoder.onnx")),
        ("projector", dir.join("projector.onnx")),
        ("embed_tokens", dir.join("embed_tokens.onnx")),
        ("editor", dir.join("editor.onnx")),
    ];

    println!("[probe] mode={mode} model_dir={}", dir.display());
    let mut sessions = Vec::new();
    let total = Instant::now();

    for (name, path) in graphs {
        let t0 = Instant::now();
        let session = match mode.as_str() {
            "cuda-lowram" => make_cuda_session(&path, true)?,
            "cuda-perf" => make_cuda_session(&path, false)?,
            "trt-fp32" => make_trt_session(&path, &cache_dir, false, false)?,
            "trt-fp16" => make_trt_session(&path, &cache_dir, true, false)?,
            "trt-fp16-cudagraph" => make_trt_session(&path, &cache_dir, true, true)?,
            other => return Err(format!("unknown mode: {other}")),
        };
        println!(
            "[probe] loaded {name} in {:.3}s",
            t0.elapsed().as_secs_f32()
        );
        sessions.push(session);
    }

    println!(
        "[probe] loaded {} sessions in {:.3}s",
        sessions.len(),
        total.elapsed().as_secs_f32()
    );
    Ok(())
}
