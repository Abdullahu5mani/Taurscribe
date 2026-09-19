// ==============================================================================
// Taurscribe ONNX Runtime CoreML Execution Provider Verification Test
// ==============================================================================

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod coreml_tests {
    use ort::ep::coreml::{ComputeUnits, ModelFormat, SpecializationStrategy};
    use ort::session::Session;
    use ort::session::builder::GraphOptimizationLevel;

    #[test]
    fn test_coreml_execution_provider_configuration() {
        println!("[COREML] Initializing CoreML Execution Provider builder...");
        
        let coreml = ort::ep::CoreML::default()
            .with_model_format(ModelFormat::MLProgram)
            .with_compute_units(ComputeUnits::All)
            .with_static_input_shapes(false)
            .with_specialization_strategy(SpecializationStrategy::FastPrediction)
            .with_low_precision_accumulation_on_gpu(true)
            .build()
            .error_on_failure();

        println!("✓ [PASS] CoreML Execution Provider successfully built with ComputeUnits::All!");

        let builder = Session::builder().expect("ORT Session builder must initialize");
        let builder = builder
            .with_execution_providers([coreml])
            .expect("Session must accept CoreML Execution Provider")
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .expect("Session must accept Level3 optimization");

        println!("✓ [PASS] ORT Session successfully registered CoreML EP targeting Apple Neural Engine & GPU!");
        let _ = builder;
    }
}
