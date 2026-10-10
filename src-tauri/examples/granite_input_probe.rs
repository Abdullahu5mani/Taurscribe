//! Diagnostic: exercise inference after a successful model load, including short file tails.
#[cfg(all(target_os="windows", feature="gpu-nvidia"))]
#[link(name="cublas")]
#[link(name="cudart")]
#[link(name="cuda")]
extern "C" {}
#[cfg(all(target_os="windows", feature="gpu-vulkan"))]
#[link(name="vulkan-1")]
extern "C" {}
fn main() {
    let path = std::env::args().nth(1).expect("model path");
    let model = transcribe_cpp::Model::load(&path).expect("model load");
    println!("arch={} backend={}", model.arch(), model.backend());
    let mut session = model.session().unwrap();
    for len in [160, 800, 1600, 3200, 8000, 16000, 16000*60] {
        let audio: Vec<f32> = (0..len).map(|i| (i as f32 * 0.1).sin()*0.01).collect();
        let result = session.run(&audio, &Default::default());
        println!("samples={len} result={:?}", result.map(|out| out.text));
    }
}
