//! Native MLX backend for Parakeet Nemotron on Apple Silicon Metal GPU.

pub mod conformer;
pub mod decoder;
pub mod engine;
pub mod subsampling;

pub use conformer::FastConformerEncoder;
pub use decoder::{JointNetwork, PredictorNetwork};
pub use engine::ParakeetNemotronMlx;
pub use subsampling::ConvSubsampling;
