//! Development smoke test for a separately supplied neural-key artifact.
//!
//! This binary is feature-gated and never participates in the application's
//! immediate classical analysis path. It loads an external ONNX Runtime
//! library, verifies the artifact contract/checksum, and executes deterministic
//! audio through schema-2 artifacts (or one legacy synthetic mel chunk).

use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use tunelock_lib::neural_key::NeuralKeySession;

fn deterministic_audio(samples: usize) -> Vec<f32> {
    let mut state = 0x5eed_1234_u32;
    (0..samples)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let signed = (state >> 8) as i32 - (1 << 23);
            signed as f32 / (1_u32 << 23) as f32
        })
        .collect()
}

fn run_probe(
    artifact_directory: std::path::PathBuf,
    runtime_dylib: std::path::PathBuf,
) -> Result<()> {
    eprintln!("loading neural-key artifact and external runtime...");
    let mut session = NeuralKeySession::load(&artifact_directory, &runtime_dylib)
        .with_context(|| format!("probing {}", artifact_directory.display()))?;
    let native_audio = session
        .artifact
        .contract
        .supports_native_myna_preprocessing();
    let (input_mode, posterior) = if native_audio {
        eprintln!("running deterministic audio through native preprocessing...");
        let samples = deterministic_audio(session.artifact.contract.input.audio_samples_per_chunk);
        ("native-16khz-audio", session.predict_audio_16khz(&samples)?)
    } else {
        eprintln!("running one legacy synthetic mel chunk...");
        let input_values = session
            .artifact
            .mel_bins
            .checked_mul(session.artifact.mel_frames)
            .context("artifact input dimensions overflowed")?;
        (
            "legacy-prepared-mel",
            session.predict_mel_chunks(vec![0.0; input_values], 1)?,
        )
    };
    let (top_index, top_value) = posterior
        .iter()
        .copied()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .context("neural-key posterior was empty")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "ok",
            "artifact": artifact_directory,
            "runtime": runtime_dylib,
            "mel_bins": session.artifact.mel_bins,
            "mel_frames": session.artifact.mel_frames,
            "input_mode": input_mode,
            "posterior_sum": posterior.iter().sum::<f32>(),
            "synthetic_top_index": top_index,
            "synthetic_top_label": session.artifact.contract.output.posterior_labels[top_index],
            "synthetic_top_probability": top_value
        }))?
    );
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        bail!("usage: tunelock-neural-key-probe <artifact-directory> <onnx-runtime-dylib>");
    }
    let artifact_directory = std::path::PathBuf::from(&arguments[0]);
    let runtime_dylib = std::path::PathBuf::from(&arguments[1]);
    // Keep the optional model away from the immediate classical-result path.
    // Production uses the same dedicated-background-worker shape.
    std::thread::Builder::new()
        .name("tunelock-neural-key-probe".to_owned())
        .spawn(move || run_probe(artifact_directory, runtime_dylib))?
        .join()
        .map_err(|_| anyhow!("neural-key probe worker panicked"))?
}
