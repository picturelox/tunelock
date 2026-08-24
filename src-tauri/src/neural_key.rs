//! Versioned neural-key artifact contract and optional ONNX Runtime adapter.
//!
//! The classical analyzer remains the unconditional, immediate result. This
//! module validates a separately supplied model artifact and, only when the
//! `neural-key` feature is enabled, can execute already-prepared mel chunks in
//! an external ONNX Runtime dylib. Audio preprocessing parity remains a
//! separate promotion gate.

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::harmony::pitch_class_to_name;

const KEY_COUNT: usize = 24;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ArtifactDimension {
    Fixed(usize),
    Dynamic(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct NeuralKeyInputContract {
    pub name: String,
    pub dtype: String,
    pub shape: Vec<ArtifactDimension>,
    pub sample_rate_hz: usize,
    pub audio_samples_per_chunk: usize,
    pub preprocessor: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NeuralKeyOutputContract {
    pub name: String,
    pub dtype: String,
    pub shape: Vec<ArtifactDimension>,
    pub posterior_labels: Vec<String>,
    pub track_aggregation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NeuralKeyArtifact {
    pub schema_version: u32,
    pub artifact_kind: String,
    pub status: String,
    pub model_file: String,
    pub model_sha256: String,
    pub model_bytes: u64,
    pub input: NeuralKeyInputContract,
    pub output: NeuralKeyOutputContract,
    pub data_rights_status: String,
}

#[derive(Debug, Clone)]
pub struct LoadedNeuralKeyArtifact {
    pub contract: NeuralKeyArtifact,
    pub model_path: PathBuf,
    pub mel_bins: usize,
    pub mel_frames: usize,
}

fn canonical_labels() -> Vec<String> {
    (0..KEY_COUNT)
        .map(|index| {
            let tonic = index % 12;
            let mode = if index < 12 { "major" } else { "minor" };
            format!("{} {mode}", pitch_class_to_name(tonic))
        })
        .collect()
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut digest = Sha256::new();
    // Keep the large streaming buffer off Windows' small main-thread stack.
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .with_context(|| format!("hashing {}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn fixed_dimension(value: &ArtifactDimension, label: &str) -> Result<usize> {
    match value {
        ArtifactDimension::Fixed(value) if *value > 0 => Ok(*value),
        _ => bail!("{label} must be a positive fixed dimension"),
    }
}

impl NeuralKeyArtifact {
    fn validate(&self) -> Result<(usize, usize)> {
        if self.schema_version != 1 || self.artifact_kind != "tunelock-neural-key-chunk-v1" {
            bail!("unsupported neural-key artifact contract");
        }
        if self.model_bytes == 0
            || self.model_sha256.len() != 64
            || !self
                .model_sha256
                .chars()
                .all(|value| value.is_ascii_hexdigit())
        {
            bail!("invalid model size or SHA-256 in neural-key artifact");
        }
        let model_path = Path::new(&self.model_file);
        if model_path.components().count() != 1
            || !matches!(model_path.components().next(), Some(Component::Normal(_)))
        {
            bail!("model_file must be one safe filename inside the artifact directory");
        }
        if self.input.name != "mel_spectrogram"
            || self.input.dtype != "float32"
            || self.input.shape.len() != 4
            || !matches!(
                &self.input.shape[0],
                ArtifactDimension::Dynamic(value) if value == "chunk_count"
            )
            || fixed_dimension(&self.input.shape[1], "input channel")? != 1
            || self.input.sample_rate_hz == 0
            || self.input.audio_samples_per_chunk == 0
            || self.input.preprocessor.trim().is_empty()
        {
            bail!("invalid neural-key input contract");
        }
        let mel_bins = fixed_dimension(&self.input.shape[2], "mel bins")?;
        let mel_frames = fixed_dimension(&self.input.shape[3], "mel frames")?;
        if self.output.name != "chunk_logits"
            || self.output.dtype != "float32"
            || self.output.shape.len() != 2
            || !matches!(
                &self.output.shape[0],
                ArtifactDimension::Dynamic(value) if value == "chunk_count"
            )
            || fixed_dimension(&self.output.shape[1], "key output")? != KEY_COUNT
            || self.output.posterior_labels != canonical_labels()
            || self.output.track_aggregation != "arithmetic mean of chunk logits, then softmax"
        {
            bail!("invalid neural-key output or harmony vocabulary contract");
        }
        if self.status.trim().is_empty() || self.data_rights_status.trim().is_empty() {
            bail!("artifact must state model and data-rights status");
        }
        Ok((mel_bins, mel_frames))
    }
}

impl LoadedNeuralKeyArtifact {
    pub fn load(directory: impl AsRef<Path>) -> Result<Self> {
        let directory = directory.as_ref();
        let manifest_path = directory.join("artifact.json");
        let contract: NeuralKeyArtifact = serde_json::from_reader(
            File::open(&manifest_path)
                .with_context(|| format!("opening {}", manifest_path.display()))?,
        )
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let (mel_bins, mel_frames) = contract.validate()?;
        let model_path = directory.join(&contract.model_file);
        let metadata = std::fs::metadata(&model_path)
            .with_context(|| format!("reading {}", model_path.display()))?;
        if !metadata.is_file() || metadata.len() != contract.model_bytes {
            bail!("neural-key model size does not match artifact manifest");
        }
        let actual_hash = sha256(&model_path)?;
        if !actual_hash.eq_ignore_ascii_case(&contract.model_sha256) {
            bail!("neural-key model SHA-256 does not match artifact manifest");
        }
        Ok(Self {
            contract,
            model_path,
            mel_bins,
            mel_frames,
        })
    }
}

/// Aggregate raw per-chunk logits using the exact training/evaluation contract.
pub fn aggregate_chunk_logits(logits: &[f32], chunk_count: usize) -> Result<[f32; KEY_COUNT]> {
    let expected = chunk_count
        .checked_mul(KEY_COUNT)
        .context("neural-key output dimensions overflowed")?;
    if chunk_count == 0 || logits.len() != expected {
        bail!("expected chunk_count * 24 finite logits");
    }
    let mut mean = [0.0_f32; KEY_COUNT];
    for chunk in logits.chunks_exact(KEY_COUNT) {
        for (target, value) in mean.iter_mut().zip(chunk) {
            if !value.is_finite() {
                bail!("neural-key logits contain a non-finite value");
            }
            *target += *value / chunk_count as f32;
        }
    }
    let maximum = mean.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut total = 0.0_f32;
    for value in &mut mean {
        *value = (*value - maximum).exp();
        total += *value;
    }
    if !total.is_finite() || total <= 0.0 {
        bail!("neural-key softmax normalization failed");
    }
    for value in &mut mean {
        *value /= total;
    }
    Ok(mean)
}

#[cfg(feature = "neural-key")]
mod runtime {
    use std::sync::Mutex;

    use anyhow::{anyhow, bail, Context, Result};
    use ort::session::{builder::GraphOptimizationLevel, Session};
    use ort::value::Tensor;

    use super::{aggregate_chunk_logits, LoadedNeuralKeyArtifact, KEY_COUNT};

    static ORT_RUNTIME: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

    fn initialize_runtime(path: &std::path::Path) -> Result<()> {
        let requested = path.to_path_buf();
        let mut loaded = ORT_RUNTIME
            .lock()
            .map_err(|_| anyhow!("ONNX Runtime initialization lock was poisoned"))?;
        match loaded.as_ref() {
            Some(existing) if existing == path => return Ok(()),
            Some(existing) => bail!(
                "ONNX Runtime is already initialized from {}, not {}",
                existing.display(),
                path.display()
            ),
            None => {}
        }
        ort::init_from(&requested)
            .map_err(|error| anyhow!("initializing ONNX Runtime: {error}"))?
            .commit();
        *loaded = Some(requested);
        Ok(())
    }

    pub struct NeuralKeySession {
        pub artifact: LoadedNeuralKeyArtifact,
        session: Session,
    }

    impl NeuralKeySession {
        pub fn load(
            artifact_directory: impl AsRef<std::path::Path>,
            runtime_dylib: impl AsRef<std::path::Path>,
        ) -> Result<Self> {
            let artifact = LoadedNeuralKeyArtifact::load(artifact_directory)?;
            let runtime_dylib = std::fs::canonicalize(runtime_dylib.as_ref())
                .with_context(|| "resolving the external ONNX Runtime library")?;
            initialize_runtime(&runtime_dylib)?;
            let session = Session::builder()
                .map_err(|error| anyhow!(error.to_string()))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                // `ort` returns the non-Send builder as part of this error;
                // erase that payload before crossing anyhow's Send + Sync boundary.
                .map_err(|error| anyhow!(error.to_string()))?
                .commit_from_file(&artifact.model_path)
                .map_err(|error| anyhow!(error.to_string()))
                .with_context(|| format!("loading {}", artifact.model_path.display()))?;
            Ok(Self { artifact, session })
        }

        /// Run prepared Myna mel chunks. Call this from a background task; a
        /// Session intentionally requires exclusive mutable access.
        pub fn predict_mel_chunks(
            &mut self,
            mel_spectrogram: Vec<f32>,
            chunk_count: usize,
        ) -> Result<[f32; KEY_COUNT]> {
            let expected = chunk_count
                .checked_mul(self.artifact.mel_bins)
                .and_then(|value| value.checked_mul(self.artifact.mel_frames))
                .context("neural-key input dimensions overflowed")?;
            if chunk_count == 0 || mel_spectrogram.len() != expected {
                bail!("mel input length does not match the artifact shape");
            }
            let input = Tensor::from_array((
                [
                    chunk_count,
                    1,
                    self.artifact.mel_bins,
                    self.artifact.mel_frames,
                ],
                mel_spectrogram.into_boxed_slice(),
            ))?;
            let outputs = self.session.run(ort::inputs!["mel_spectrogram" => input])?;
            let (_, logits) = outputs["chunk_logits"].try_extract_tensor::<f32>()?;
            aggregate_chunk_logits(logits, chunk_count)
        }
    }
}

#[cfg(feature = "neural-key")]
pub use runtime::NeuralKeySession;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_contract_is_major_then_minor() {
        let labels = canonical_labels();
        assert_eq!(labels.len(), 24);
        assert_eq!(labels[0], "C major");
        assert_eq!(labels[11], "B major");
        assert_eq!(labels[12], "C minor");
        assert_eq!(labels[23], "B minor");
    }

    #[test]
    fn chunk_logits_are_meaned_before_softmax() {
        let mut logits = vec![0.0_f32; KEY_COUNT * 2];
        logits[3] = 2.0;
        logits[KEY_COUNT + 3] = 4.0;
        let posterior = aggregate_chunk_logits(&logits, 2).unwrap();
        assert_eq!(
            posterior
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.partial_cmp(right.1).unwrap())
                .unwrap()
                .0,
            3
        );
        assert!((posterior.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn invalid_logit_shapes_and_values_are_rejected() {
        assert!(aggregate_chunk_logits(&[], 0).is_err());
        assert!(aggregate_chunk_logits(&[0.0; KEY_COUNT - 1], 1).is_err());
        let mut invalid = [0.0_f32; KEY_COUNT];
        invalid[4] = f32::NAN;
        assert!(aggregate_chunk_logits(&invalid, 1).is_err());
    }

    #[test]
    fn artifact_load_checks_contract_size_and_checksum() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "tunelock-neural-key-artifact-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let model_path = directory.join("key-model.onnx");
        std::fs::write(&model_path, b"model").unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "artifact_kind": "tunelock-neural-key-chunk-v1",
            "status": "test-only",
            "model_file": "key-model.onnx",
            "model_sha256": sha256(&model_path).unwrap(),
            "model_bytes": 5,
            "input": {
                "name": "mel_spectrogram",
                "dtype": "float32",
                "shape": ["chunk_count", 1, 128, 196],
                "sample_rate_hz": 16000,
                "audio_samples_per_chunk": 100000,
                "preprocessor": "test fixture"
            },
            "output": {
                "name": "chunk_logits",
                "dtype": "float32",
                "shape": ["chunk_count", 24],
                "posterior_labels": canonical_labels(),
                "track_aggregation": "arithmetic mean of chunk logits, then softmax"
            },
            "data_rights_status": "test fixture"
        });
        std::fs::write(
            directory.join("artifact.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let artifact = LoadedNeuralKeyArtifact::load(&directory).unwrap();
        assert_eq!(artifact.mel_bins, 128);
        assert_eq!(artifact.mel_frames, 196);
        std::fs::write(&model_path, b"other").unwrap();
        assert!(LoadedNeuralKeyArtifact::load(&directory).is_err());

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
