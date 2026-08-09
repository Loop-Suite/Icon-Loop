use crate::models::{CritiqueRound, IconCandidate, PolicyReport, QuantResult};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub round: usize,
    pub spec_name: String,
    /// Accumulates the raw text of dead concepts and failure reasons. A new agent is spawned each round,
    /// but lessons are passed down through this text (Reflexion/GEPA-style — external memory accumulation rather than persisting agent identity).
    pub dead_concepts: String,
    pub candidates: Vec<IconCandidate>,
    pub policies: Vec<PolicyReport>,
    pub discourse: Vec<CritiqueRound>,
    pub quant: QuantResult,
}

pub fn write(out_dir: &Path, state: &State) -> Result<()> {
    let path = out_dir.join("state.json");
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write state.json: {}", path.display()))?;
    Ok(())
}

pub fn load(run_dir: &Path) -> Result<State> {
    let path = if run_dir.is_dir() {
        run_dir.join("state.json")
    } else {
        run_dir.to_path_buf()
    };
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read state.json: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse state.json: {}", path.display()))
}
