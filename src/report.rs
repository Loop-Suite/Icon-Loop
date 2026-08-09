use crate::models::{CritiqueRound, IconCandidate, PolicyReport, QuantResult};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn write(
    out_dir: &Path,
    round: usize,
    candidates: &[IconCandidate],
    policies: &[PolicyReport],
    discourse: &[CritiqueRound],
    quant: &QuantResult,
) -> Result<PathBuf> {
    let mut md = String::new();
    md.push_str(&format!("# icon-loop Run Report (round {round})\n\n"));

    let winner_persona = candidates
        .iter()
        .find(|c| c.id == quant.winner)
        .map(|c| c.persona_name.as_str())
        .unwrap_or("?");
    let winner_score = quant.borda.first().map(|(_, s)| *s).unwrap_or(0);
    md.push_str(&format!(
        "**Verdict: {} — {} (Borda {}pt)**\n\n",
        quant.winner, winner_persona, winner_score
    ));
    md.push_str(&format!("Judging panel: {}\n\n", quant.critic_diversity_note));
    if let Some(warning) = &quant.unanimous_warning {
        md.push_str(&format!("⚠️ {warning}\n\n"));
    }
    if !quant.minority_opinions.is_empty() {
        md.push_str("**Minority Opinions (overridden in Borda):**\n\n");
        for note in &quant.minority_opinions {
            md.push_str(&format!("- {note}\n"));
        }
        md.push('\n');
    }

    md.push_str("## Borda Tally (deterministic, no LLM involved)\n\n| Rank | candidate | Persona | Score |\n|---|---|---|---|\n");
    for (rank, (id, score)) in quant.borda.iter().enumerate() {
        let persona = candidates
            .iter()
            .find(|c| &c.id == id)
            .map(|c| c.persona_name.as_str())
            .unwrap_or("?");
        md.push_str(&format!("| {} | {id} | {persona} | {score} |\n", rank + 1));
    }

    md.push_str("\n## Deterministic Gate (policy.rs, no LLM involved)\n\n");
    for policy in policies {
        md.push_str(&format!(
            "### {} — {}\n\n",
            policy.candidate_id,
            if policy.overall_pass { "PASS" } else { "FAIL" }
        ));
        for check in &policy.checks {
            md.push_str(&format!("- [{}] {}: {}\n", check.status.label(), check.id, check.evidence));
        }
        md.push('\n');
    }

    md.push_str("## Discourse — Independent Blind Critics (cannot see each other)\n\n");
    for round_result in discourse {
        md.push_str(&format!(
            "### Critic {} (provider={})\n\n",
            round_result.critic_index + 1,
            round_result.provider
        ));
        for item in &round_result.items {
            md.push_str(&format!(
                "**{}**\n- Blind first impression: {}\n- Category signal: {}\n- {} legibility: {}\n- Flaw: {}\n\n",
                item.candidate_id,
                item.blind_read,
                item.category_signal,
                "small",
                item.legibility_29px,
                item.biggest_flaw
            ));
        }
        md.push_str(&format!("Ranking: {}\n\n", round_result.ranking.join(" > ")));
    }

    let path = out_dir.join("report.md");
    std::fs::write(&path, &md).with_context(|| format!("failed to write report.md: {}", path.display()))?;
    Ok(path)
}
