//! Proof layer — ground-truth corpora and accuracy scoring.
//!
//! This module knows nothing about Tauri, the UI, or the database. It turns
//! an exported Mixed In Key CSV (or any compatible corpus) into normalised
//! labels, and turns (prediction, truth) pairs into MIREX-weighted scores and
//! an error-type taxonomy. The `tunelock-bench` binary drives it.
//!
//! Design rule: MIK labels are a *reference opinion*, not truth. They power
//! disagreement triage and regression detection. Absolute accuracy claims come
//! from GiantSteps and human-adjudicated gold.

pub mod corpus;
pub mod metrics;
