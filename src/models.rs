use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IconCandidate {
    pub id: String,
    pub persona_id: String,
    pub persona_name: String,
    pub svg: String,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum PolicyStatus {
    Pass,
    Fail,
}

impl PolicyStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicyCheck {
    pub id: String,
    pub status: PolicyStatus,
    pub evidence: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicyReport {
    pub candidate_id: String,
    pub checks: Vec<PolicyCheck>,
    pub fg_ratio: f32,
    pub overall_pass: bool,
    pub render_paths: Vec<(u32, String)>,
}

/// Result of a single critic (one independent call) blindly evaluating all candidates.
/// Generated independently, without seeing other critics' results — see discourse.rs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CritiqueRound {
    pub critic_index: usize,
    /// Which provider (claude/openrouter) this critic actually used to judge — for auditing panel diversity.
    pub provider: String,
    pub items: Vec<CandidateCritique>,
    /// List of candidate_id in order starting from 1st place (no ties; decided directly by the critic)
    pub ranking: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CandidateCritique {
    pub candidate_id: String,
    pub blind_read: String,
    /// Whether the candidate's visual metaphor actually matches the app's domain, as described in
    /// `Spec.context` (see `discourse::round_prompt`) — generalized on purpose. An earlier version of
    /// the prompt hardcoded this to "whether it signals a fortune-telling/mysticism app", which was
    /// only ever accidentally correct because the shipped example spec is a divination-app brief; for
    /// any other domain it forced an unrelated evaluation criterion onto every critic.
    pub category_signal: String,
    pub legibility_29px: String,
    pub biggest_flaw: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuantResult {
    /// candidate_id -> Borda score (computed purely in code, no LLM involved)
    pub borda: Vec<(String, u32)>,
    pub winner: String,
    /// Audit of the critic panel's provider composition — carries a strong warning if all critics share the same provider.
    pub critic_diversity_note: String,
    /// Warns when critic rankings are perfectly identical (unanimous) — possible collusion/groupthink (per research notes,
    /// Towards AI: "a 100% critique pass rate signals collusion, not excellence").
    pub unanimous_warning: Option<String>,
    /// Marks candidates that ranked last overall in Borda but were ranked first by at least one critic — flags
    /// minority opinions buried by groupthink (based on strangeloopcanon's "LLM Councils Show Groupthink" experiment).
    pub minority_opinions: Vec<String>,
}
