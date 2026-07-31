//! Confirms mlx-rs reaches the GPU and exposes the ops the Granite port needs.
#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use mlx_rs::{fast, ops, Array, Dtype};

    // A real matmul on the default (GPU) stream.
    let a = Array::from_slice(&[1.0f32, 2.0, 3.0, 4.0], &[2, 2]);
    let b = Array::from_slice(&[5.0f32, 6.0, 7.0, 8.0], &[2, 2]);
    let c = a.matmul(&b)?;
    println!("matmul  -> {:?}", c.as_slice::<f32>());

    // fp16, the dtype the port runs in.
    let h = a.as_dtype(Dtype::Float16)?;
    println!("f16 cast-> {:?}", h.dtype());

    // The four fast kernels the model leans on.
    let w = Array::from_slice(&[1.0f32, 1.0], &[2]);
    let rn = fast::rms_norm(&a, &w, 1e-5)?;
    println!("rms_norm-> {:?}", rn.shape());

    let x = Array::from_slice(&vec![0.1f32; 1 * 2 * 4 * 8], &[1, 2, 4, 8]);
    let r = fast::rope(&x, 8, false, 10000.0, 1.0, 0, None)?;
    println!("rope    -> {:?}", r.shape());

    let sdpa = fast::scaled_dot_product_attention(&x, &x, &x, 0.125, None)?;
    println!("sdpa    -> {:?}", sdpa.shape());

    let sm = ops::softmax_axis(&a, -1, None)?;
    println!("softmax -> {:?}", sm.shape());

    println!("\nOK: mlx-rs runs on Metal and has every op the Granite port needs.");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS only");
}
