//! Export key-training/evaluation manifests with canonical labels from Rust.
//!
//! Python experiment code consumes numeric targets from this artifact. It does
//! not parse key names, keeping TuneLock's existing harmony vocabulary as the
//! single source of truth.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tunelock_lib::analysis::pitch_class_to_name;
use tunelock_lib::proof::corpus::{load_giantsteps, parse_standard_key, RowStatus};

const KEY_COUNT: usize = 24;

#[derive(Debug)]
struct Args {
    mtg: PathBuf,
    giantsteps: PathBuf,
    out: PathBuf,
    mtg_protocol: MtgProtocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MtgProtocol {
    Published1077,
    AllUnambiguous,
}

impl MtgProtocol {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "published-1077" => Ok(Self::Published1077),
            "all-unambiguous" => Ok(Self::AllUnambiguous),
            other => {
                bail!("unknown MTG protocol {other:?}; expected published-1077 or all-unambiguous")
            }
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Published1077 => "MTG confidence=2, unambiguous single-key labels, and empty annotator comment (published 1,077-track subset); GiantSteps-key is development-only",
            Self::AllUnambiguous => "MTG all-confidence, unambiguous single-key labels (KeyMyna 1,486-corpus ablation); GiantSteps-key is development-only",
        }
    }
}

fn parse_args() -> Result<Args> {
    let mut mtg = None;
    let mut giantsteps = None;
    let mut out = None;
    let mut mtg_protocol = MtgProtocol::Published1077;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mtg" => mtg = args.next().map(PathBuf::from),
            "--giantsteps" => giantsteps = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--mtg-protocol" => {
                mtg_protocol =
                    MtgProtocol::parse(&args.next().context("--mtg-protocol requires a value")?)?
            }
            "--help" | "-h" => {
                println!(
                    "Usage: tunelock-key-corpus-manifest --mtg <root> --giantsteps <root> \
                     --out <manifest.json> [--mtg-protocol published-1077|all-unambiguous]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok(Args {
        mtg: mtg.context("--mtg is required")?,
        giantsteps: giantsteps.context("--giantsteps is required")?,
        out: out.context("--out is required")?,
        mtg_protocol,
    })
}

#[derive(Debug, Deserialize)]
struct MtgAnnotation {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "MANUAL KEY")]
    manual_key: String,
    #[serde(rename = "C")]
    confidence: String,
}

#[derive(Debug, Deserialize)]
struct MtgMetadata {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "ARTIST")]
    artist: String,
}

#[derive(Debug, Serialize)]
struct ManifestRecord {
    corpus: &'static str,
    role: &'static str,
    id: String,
    audio_path: String,
    truth_index: usize,
    truth_label: String,
    confidence: Option<u8>,
    artist: String,
    genre: String,
    recording_md5: Option<String>,
}

#[derive(Debug, Serialize)]
struct ExclusionCounts {
    mtg_not_high_confidence: usize,
    mtg_ambiguous_or_invalid_key: usize,
    mtg_annotator_comment: usize,
    mtg_missing_audio: usize,
}

#[derive(Debug, Serialize)]
struct Manifest {
    schema_version: u32,
    canonical_labels: Vec<String>,
    pitch_shift_targets: Vec<PitchShiftTargets>,
    training_protocol: &'static str,
    train_records: usize,
    development_records: usize,
    exclusions: ExclusionCounts,
    records: Vec<ManifestRecord>,
}

#[derive(Debug, Serialize)]
struct PitchShiftTargets {
    semitones: i32,
    target_by_source_index: Vec<usize>,
}

fn key_index(tonic: usize, is_major: bool) -> usize {
    tonic + if is_major { 0 } else { 12 }
}

fn transpose_index(index: usize, semitones: i32) -> usize {
    let tonic = index % 12;
    let mode_offset = if index < 12 { 0 } else { 12 };
    (tonic as i32 + semitones).rem_euclid(12) as usize + mode_offset
}

fn canonical_label(tonic: usize, is_major: bool) -> String {
    format!(
        "{} {}",
        pitch_class_to_name(tonic),
        if is_major { "major" } else { "minor" }
    )
}

fn first_token(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_lowercase))
}

fn parse_track_annotation(text: &str) -> Result<(&str, &str, &str)> {
    let mut fields = text.trim_end_matches(['\r', '\n']).splitn(3, '\t');
    let key = fields.next().unwrap_or_default().trim();
    let confidence = fields.next().unwrap_or_default().trim();
    let comment = fields.next().unwrap_or_default().trim();
    if key.is_empty() || confidence.is_empty() {
        bail!("track annotation must contain tab-separated key and confidence fields");
    }
    Ok((key, confidence, comment))
}

fn load_mtg(root: &Path, protocol: MtgProtocol) -> Result<(Vec<ManifestRecord>, ExclusionCounts)> {
    let annotations_path = root.join("annotations").join("annotations.txt");
    let metadata_path = root.join("annotations").join("beatport_metadata.txt");

    let mut metadata_reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_path(&metadata_path)
        .with_context(|| format!("opening {}", metadata_path.display()))?;
    let mut artists = HashMap::new();
    for row in metadata_reader.deserialize::<MtgMetadata>() {
        let row = row.with_context(|| format!("parsing {}", metadata_path.display()))?;
        artists.insert(row.id, row.artist.trim().to_string());
    }

    let mut annotation_reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_path(&annotations_path)
        .with_context(|| format!("opening {}", annotations_path.display()))?;

    let mut records = Vec::new();
    let mut excluded_confidence = 0;
    let mut excluded_key = 0;
    let mut excluded_comment = 0;
    let mut excluded_audio = 0;
    for row in annotation_reader.deserialize::<MtgAnnotation>() {
        let row = row.with_context(|| format!("parsing {}", annotations_path.display()))?;
        if protocol == MtgProtocol::Published1077 && row.confidence.trim() != "2" {
            excluded_confidence += 1;
            continue;
        }

        let key = row.manual_key.trim();
        let Some((tonic, is_major)) = parse_standard_key(key) else {
            // Ambiguous annotations (for example "C minor / F# minor") are
            // intentionally excluded instead of collapsed to an arbitrary key.
            excluded_key += 1;
            continue;
        };

        let stem = format!("{}.LOFI", row.id.trim());
        let track_annotation_path = root
            .join("annotations")
            .join("key")
            .join(format!("{stem}.key"));
        let track_annotation_text = std::fs::read_to_string(&track_annotation_path)
            .with_context(|| format!("opening {}", track_annotation_path.display()))?;
        let (track_key, track_confidence, comment) = parse_track_annotation(&track_annotation_text)
            .with_context(|| format!("parsing {}", track_annotation_path.display()))?;
        if !track_key.eq_ignore_ascii_case(key) || track_confidence != row.confidence.trim() {
            bail!(
                "aggregate/per-track annotation mismatch for {stem}: aggregate=({key:?}, {:?}), track=({track_key:?}, {track_confidence:?})",
                row.confidence.trim()
            );
        }
        // This reproduces the published 1,077-track GiantSteps-MTG protocol:
        // confidence 2, one parseable key, and no annotator qualification.
        if protocol == MtgProtocol::Published1077 && !comment.is_empty() {
            excluded_comment += 1;
            continue;
        }

        let audio = root.join("audio").join(format!("{stem}.mp3"));
        if !audio.is_file() {
            excluded_audio += 1;
            continue;
        }
        let genre = std::fs::read_to_string(
            root.join("annotations")
                .join("genre")
                .join(format!("{stem}.genre")),
        )
        .unwrap_or_default()
        .trim()
        .to_string();
        let recording_md5 = first_token(&root.join("md5").join(format!("{stem}.md5")));

        records.push(ManifestRecord {
            corpus: "giantsteps-mtg-key",
            role: "training",
            id: stem,
            audio_path: audio.to_string_lossy().to_string(),
            truth_index: key_index(tonic, is_major),
            truth_label: canonical_label(tonic, is_major),
            confidence: row.confidence.trim().parse().ok(),
            artist: artists.get(row.id.trim()).cloned().unwrap_or_default(),
            genre,
            recording_md5,
        });
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    Ok((
        records,
        ExclusionCounts {
            mtg_not_high_confidence: excluded_confidence,
            mtg_ambiguous_or_invalid_key: excluded_key,
            mtg_annotator_comment: excluded_comment,
            mtg_missing_audio: excluded_audio,
        },
    ))
}

fn load_development(root: &Path) -> Vec<ManifestRecord> {
    let mut records = Vec::new();
    for row in load_giantsteps(root) {
        if row.status != RowStatus::Ready {
            continue;
        }
        let (Some(tonic), Some(is_major)) = (row.truth_tonic, row.truth_is_major) else {
            continue;
        };
        let audio = PathBuf::from(&row.location);
        let recording_md5 = first_token(&root.join("md5").join(format!("{}.md5", row.title)));
        records.push(ManifestRecord {
            corpus: "giantsteps-key",
            role: "development",
            id: row.title,
            audio_path: audio.to_string_lossy().to_string(),
            truth_index: key_index(tonic, is_major),
            truth_label: canonical_label(tonic, is_major),
            confidence: None,
            artist: row.artist,
            genre: row.genre,
            recording_md5,
        });
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    records
}

fn run(args: Args) -> Result<()> {
    let (mut training, exclusions) = load_mtg(&args.mtg, args.mtg_protocol)?;
    let development = load_development(&args.giantsteps);
    if training.is_empty() || development.is_empty() {
        bail!(
            "manifest would be empty: training={}, development={}",
            training.len(),
            development.len()
        );
    }

    let train_records = training.len();
    let development_records = development.len();
    training.extend(development);
    let manifest = Manifest {
        schema_version: 1,
        canonical_labels: (0..KEY_COUNT)
            .map(|index| canonical_label(index % 12, index < 12))
            .collect(),
        pitch_shift_targets: (-6..=6)
            .map(|semitones| PitchShiftTargets {
                semitones,
                target_by_source_index: (0..KEY_COUNT)
                    .map(|index| transpose_index(index, semitones))
                    .collect(),
            })
            .collect(),
        training_protocol: args.mtg_protocol.description(),
        train_records,
        development_records,
        exclusions,
        records: training,
    };

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    serde_json::to_writer_pretty(
        File::create(&args.out).with_context(|| format!("creating {}", args.out.display()))?,
        &manifest,
    )
    .with_context(|| format!("writing {}", args.out.display()))?;
    println!(
        "Manifest written: {} training + {} development; excluded {} low-confidence, {} ambiguous, {} commented, {} missing audio",
        train_records,
        development_records,
        manifest.exclusions.mtg_not_high_confidence,
        manifest.exclusions.mtg_ambiguous_or_invalid_key,
        manifest.exclusions.mtg_annotator_comment,
        manifest.exclusions.mtg_missing_audio
    );
    Ok(())
}

fn main() -> Result<()> {
    run(parse_args()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_order_matches_major_then_minor_contract() {
        assert_eq!(key_index(0, true), 0);
        assert_eq!(key_index(11, true), 11);
        assert_eq!(key_index(0, false), 12);
        assert_eq!(key_index(11, false), 23);
    }

    #[test]
    fn canonical_labels_round_trip_through_shared_parser() {
        for index in 0..KEY_COUNT {
            let label = canonical_label(index % 12, index < 12);
            let (tonic, major) = parse_standard_key(&label).unwrap();
            assert_eq!(key_index(tonic, major), index);
        }
    }

    #[test]
    fn pitch_transposition_preserves_mode_and_wraps_tonic() {
        assert_eq!(transpose_index(key_index(0, true), -1), key_index(11, true));
        assert_eq!(
            transpose_index(key_index(11, false), 2),
            key_index(1, false)
        );
        assert_eq!(transpose_index(key_index(6, true), 12), key_index(6, true));
    }

    #[test]
    fn track_annotation_preserves_optional_comment() {
        assert_eq!(
            parse_track_annotation("d major\t2\t\r\n").unwrap(),
            ("d major", "2", "")
        );
        assert_eq!(
            parse_track_annotation("F# minor\t2\tmodulates near outro\n").unwrap(),
            ("F# minor", "2", "modulates near outro")
        );
    }

    #[test]
    fn mtg_protocol_names_are_explicit() {
        assert_eq!(
            MtgProtocol::parse("published-1077").unwrap(),
            MtgProtocol::Published1077
        );
        assert_eq!(
            MtgProtocol::parse("all-unambiguous").unwrap(),
            MtgProtocol::AllUnambiguous
        );
        assert!(MtgProtocol::parse("all").is_err());
    }
}
