// Borda count aggregation — no LLM involved, pure code. Separating the boundary between
// critique judgment (discourse) and score calculation (quantify) is a core principle across Loop-Suite (see the codedesign-loop README).
//
// Incorporates research: (1) Warn when all judges share the same provider (arXiv:2605.29800 — high judge
// correlation means N judges effectively collapse into far fewer independent votes). (2) Warn when the ranking is unanimous (Towards AI blog
// — "a 100% critique pass rate signals collusion, not excellence"). (3) Even if a candidate ranks last overall in Borda, flag it separately
// if a specific critic ranked it first (strangeloopcanon "LLM Councils Show Groupthink" — peer-review
// aggregation has been observed to suppress original proposals backed by only a minority).
use crate::models::{CritiqueRound, QuantResult};
use std::collections::{HashMap, HashSet};

pub fn borda_count(candidate_ids: &[String], rounds: &[CritiqueRound]) -> QuantResult {
    let n = candidate_ids.len() as i64;
    let mut scores: HashMap<String, u32> = candidate_ids.iter().map(|id| (id.clone(), 0)).collect();

    for round in rounds {
        for (pos, id) in round.ranking.iter().enumerate() {
            if let Some(score) = scores.get_mut(id) {
                let points = (n - pos as i64).max(0) as u32;
                *score += points;
            }
        }
    }

    let mut borda: Vec<(String, u32)> = scores.into_iter().collect();
    borda.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let winner = borda.first().map(|(id, _)| id.clone()).unwrap_or_default();

    let critic_diversity_note = diversity_note(rounds);
    let unanimous_warning = unanimous_warning(rounds);
    let minority_opinions = minority_opinions(&borda, rounds);

    QuantResult {
        borda,
        winner,
        critic_diversity_note,
        unanimous_warning,
        minority_opinions,
    }
}

fn diversity_note(rounds: &[CritiqueRound]) -> String {
    let providers: HashSet<&str> = rounds.iter().map(|r| r.provider.as_str()).collect();
    if providers.len() <= 1 {
        format!(
            "Warning: all {} critics use provider=\"{}\" — the judging panel is effectively repeated calls to the same model, \
             which weakens the premise of independent judgment (arXiv:2605.29800). critic_backends should mix different providers.",
            rounds.len(),
            providers.into_iter().next().unwrap_or("?")
        )
    } else {
        let mut list: Vec<&str> = providers.into_iter().collect();
        list.sort();
        format!(
            "Judging panel mixes {} providers: {}",
            list.len(),
            list.join(", ")
        )
    }
}

fn unanimous_warning(rounds: &[CritiqueRound]) -> Option<String> {
    if rounds.len() < 2 {
        return None;
    }
    let first = &rounds[0].ranking;
    let all_same = rounds.iter().all(|r| r.ranking == *first);
    if all_same {
        Some(
            "All critics produced an identical ranking — check for possible collusion/groupthink. \
             If the judging panel used diverse providers, unanimity increases confidence in the result; but if all providers are the same, \
             reinterpret this alongside the critic_diversity_note warning."
                .to_string(),
        )
    } else {
        None
    }
}

fn minority_opinions(borda: &[(String, u32)], rounds: &[CritiqueRound]) -> Vec<String> {
    let Some((winner_id, _)) = borda.first() else {
        return Vec::new();
    };
    let mut notes = Vec::new();
    for (candidate_id, _) in borda.iter().skip(1) {
        let first_place_critics: Vec<usize> = rounds
            .iter()
            .filter(|r| r.ranking.first() == Some(candidate_id))
            .map(|r| r.critic_index)
            .collect();
        if !first_place_critics.is_empty() {
            notes.push(format!(
                "{candidate_id}: ranks low in Borda but was ranked first by {} critic(s) ({:?}) — a minority opinion overridden by majority vote; \
                 consider grafting this strength onto other personas in the next round",
                first_place_critics.len(),
                first_place_critics
            ));
        }
    }
    let _ = winner_id;
    notes
}
