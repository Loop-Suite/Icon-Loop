use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Persona {
    pub id: String,
    pub persona_name: String,
    pub philosophy: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Spec {
    pub name: String,
    #[serde(default)]
    pub context: String,
    pub canvas: u32,
    pub margin_ratio: f32,
    pub render_sizes: Vec<u32>,
    pub legibility_size: u32,
    pub min_fg_ratio: f32,
    pub max_fg_ratio: f32,
    pub background_hex: String,
    pub palette: Vec<String>,
    pub n_critics: usize,
    /// Critic i uses critic_backends[i % len] — "claude" or "openrouter".
    /// If all backends are the same, discourse's "independent judgment" premise weakens (per research notes,
    /// arXiv:2605.29800 — high judge correlation means N judges effectively collapse into far fewer independent votes).
    #[serde(default = "default_critic_backends")]
    pub critic_backends: Vec<String>,
    #[serde(default = "default_openrouter_critic_model")]
    pub openrouter_critic_model: String,
    pub personas: Vec<Persona>,
}

fn default_critic_backends() -> Vec<String> {
    vec!["claude".to_string()]
}

fn default_openrouter_critic_model() -> String {
    "x-ai/grok-4.5".to_string()
}

/// Upper bound (px, per dimension) for `canvas`/`render_sizes`/`legibility_size`. `render::render_all`
/// allocates a `width * height * 4` byte buffer per render size with no other cap — well below this
/// bound (e.g. 50_000px) that's already a ~10GB allocation, which aborts the process rather than
/// failing gracefully. 8192px is far beyond any real app-icon use case (the shipped example spec
/// uses 1024) while leaving generous headroom for large source art.
const MAX_RENDER_DIMENSION: u32 = 8192;

/// Upper bound on `n_critics` / `personas.len()` — both directly control how many real (billed) LLM
/// calls a single `iconloop design`/`refine` invocation makes. Without a cap, a typo (extra zero) or
/// a malicious spec turns one invocation into an unbounded number of API calls with no confirmation
/// step. 50 is far beyond any real use case (the shipped example spec uses 3 of each).
const MAX_CALL_COUNT: usize = 50;

impl Spec {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read spec file: {}", path.display()))?;
        let spec: Spec = toml::from_str(&raw)
            .with_context(|| format!("failed to parse spec TOML: {}", path.display()))?;
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.personas.is_empty(), "personas is empty");
        anyhow::ensure!(!self.palette.is_empty(), "palette is empty");
        anyhow::ensure!(self.n_critics >= 1, "n_critics must be at least 1");
        anyhow::ensure!(
            self.n_critics <= MAX_CALL_COUNT,
            "n_critics too large ({}, max {MAX_CALL_COUNT}) — would trigger an unbounded number of LLM calls",
            self.n_critics
        );
        anyhow::ensure!(
            self.personas.len() <= MAX_CALL_COUNT,
            "too many personas ({}, max {MAX_CALL_COUNT}) — would trigger an unbounded number of LLM calls",
            self.personas.len()
        );
        anyhow::ensure!(
            (1..=MAX_RENDER_DIMENSION).contains(&self.canvas),
            "canvas out of range ({}, must be 1..={MAX_RENDER_DIMENSION})",
            self.canvas
        );
        anyhow::ensure!(
            (1..=MAX_RENDER_DIMENSION).contains(&self.legibility_size),
            "legibility_size out of range ({}, must be 1..={MAX_RENDER_DIMENSION})",
            self.legibility_size
        );
        anyhow::ensure!(!self.render_sizes.is_empty(), "render_sizes is empty");
        for &size in &self.render_sizes {
            anyhow::ensure!(
                (1..=MAX_RENDER_DIMENSION).contains(&size),
                "render_sizes entry out of range ({size}, must be 1..={MAX_RENDER_DIMENSION})"
            );
        }
        Ok(())
    }

    pub fn margin_px(&self) -> f32 {
        self.canvas as f32 * self.margin_ratio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> Spec {
        Spec {
            name: "test".to_string(),
            context: String::new(),
            canvas: 100,
            margin_ratio: 0.10,
            render_sizes: vec![100, 29],
            legibility_size: 29,
            min_fg_ratio: 0.0,
            max_fg_ratio: 1.0,
            background_hex: "#000000".to_string(),
            palette: vec!["#ffffff".to_string()],
            n_critics: 1,
            critic_backends: vec!["claude".to_string()],
            openrouter_critic_model: "x-ai/grok-4.5".to_string(),
            personas: vec![Persona {
                id: "p1".to_string(),
                persona_name: "Persona One".to_string(),
                philosophy: "test".to_string(),
            }],
        }
    }

    #[test]
    fn valid_spec_passes_validation() {
        valid_spec()
            .validate()
            .expect("baseline fixture must be valid");
    }

    #[test]
    fn rejects_oversized_canvas() {
        let mut spec = valid_spec();
        spec.canvas = 50_000; // would be a ~10GB Pixmap allocation
        assert!(spec.validate().is_err());
    }

    #[test]
    fn rejects_oversized_render_sizes_entry() {
        let mut spec = valid_spec();
        spec.render_sizes = vec![100, 4_000_000_000];
        assert!(spec.validate().is_err());
    }

    #[test]
    fn rejects_empty_render_sizes() {
        let mut spec = valid_spec();
        spec.render_sizes = vec![];
        assert!(spec.validate().is_err());
    }

    #[test]
    fn rejects_too_many_personas() {
        let mut spec = valid_spec();
        spec.personas = (0..1000)
            .map(|i| Persona {
                id: format!("p{i}"),
                persona_name: "x".to_string(),
                philosophy: "x".to_string(),
            })
            .collect();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn rejects_n_critics_too_large() {
        let mut spec = valid_spec();
        spec.n_critics = 100_000;
        assert!(spec.validate().is_err());
    }

    #[test]
    fn empty_spec_file_fails_to_load_cleanly_not_panic() {
        // An empty (or otherwise garbage) spec file is missing every required field — this must
        // surface as a normal `Result::Err` from `Spec::load`, not a panic, since `--spec` can point
        // at an arbitrary user-supplied path.
        let tmp = std::env::temp_dir().join(format!(
            "icon-loop-spec-test-empty-{}.toml",
            std::process::id()
        ));
        std::fs::write(&tmp, "").expect("write temp spec file");
        let result = Spec::load(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert!(
            result.is_err(),
            "empty spec file must fail to load, not panic"
        );
    }
}
