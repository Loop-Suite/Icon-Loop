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

impl Spec {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read spec file: {}", path.display()))?;
        let spec: Spec = toml::from_str(&raw)
            .with_context(|| format!("failed to parse spec TOML: {}", path.display()))?;
        anyhow::ensure!(!spec.personas.is_empty(), "personas is empty");
        anyhow::ensure!(!spec.palette.is_empty(), "palette is empty");
        anyhow::ensure!(spec.n_critics >= 1, "n_critics must be at least 1");
        Ok(spec)
    }

    pub fn margin_px(&self) -> f32 {
        self.canvas as f32 * self.margin_ratio
    }
}
