use crate::parakeet::LoadedModel;

/// Best-effort holder for the full Parakeet model object graph.
///
/// `parakeet-rs` owns its ORT sessions internally, so unloading means dropping
/// the entire loaded model wrapper and letting the crate release those sessions.
pub struct LoadedParakeetRuntime {
    pub generation: u64,
    pub model: LoadedModel,
}
