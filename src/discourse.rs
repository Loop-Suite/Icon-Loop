// N anonymous blind critics — unlike codedesign-loop's sequential discourse (each round
// accumulates on the previous round's opinions), here the critics are completely independent
// calls that never see one another.
// Rationale: "Debate or Vote" (arXiv:2508.17536) — most of the performance gain attributed to
// multi-agent debate actually comes from the ensemble/aggregation effect, not the debate itself.
// Real-time cross-exposure debate only amplifies sycophancy risk (arXiv:2510.07517; anonymization
// drops the conformity gap from 0.608 to 0.024). So quantify.rs's deterministic Borda count owns
// the aggregation, and this module only handles "judge independently, without seeing others."
//
// The prompt is backend-agnostic: the Claude CLI sees the path and opens it directly with the
// Read tool, while OpenRouter has llm.rs base64-encode the same image file and attach it directly
// to the message — either way, the critic ends up "seeing the image."
use crate::llm::Llm;
use crate::models::{CandidateCritique, CritiqueRound};
use crate::spec::Spec;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const SYSTEM: &str = "You are a critic anonymously evaluating app icon candidates. \
You have no idea which persona created which candidate — you judge based solely on the rendered images. \
Assume no other critic's opinion exists. You must respond only in the specified JSON schema.";

#[derive(Debug, serde::Deserialize)]
struct RawResponse {
    items: Vec<CandidateCritique>,
    ranking: Vec<String>,
}

fn round_prompt(spec: &Spec, blind_ids: &[String], image_dir: &Path) -> String {
    // The catalog text must describe exactly the images run_one() actually attaches (one per
    // spec.render_sizes entry, per candidate) — a prior version hardcoded "3 images" (canvas /
    // 60px / legibility_size) here while run_one() attached one image per spec.render_sizes entry
    // (4 by default), so the OpenRouter backend — which has no filename metadata on attached
    // images and must infer candidate boundaries from the stated count — silently mispaired every
    // candidate after the first. Deriving both from spec.render_sizes keeps them in lockstep.
    let catalog = blind_ids
        .iter()
        .map(|id| {
            let paths = spec
                .render_sizes
                .iter()
                .map(|&size| {
                    format!(
                        "    {}",
                        image_dir.join(format!("{id}_{size}.png")).display()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("- {id}:\n{paths}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let sizes_label = spec
        .render_sizes
        .iter()
        .map(|size| format!("{size}px"))
        .collect::<Vec<_>>()
        .join(" / ");

    format!(
        "## Product context\n{}\n\n\
         ## Task\nCheck {} image(s) per candidate ({sizes_label}) — they're either attached \
         directly as vision input in the message, or available at the paths below via the Read tool \
         (either way, actually look at them). Since you haven't been told the intent, answer with \
         your pure first impression first.\n\n\
         {catalog}\n\n\
         Fill in the following for each candidate:\n\
         - blind_read: your first impression of what it looks like from the large size alone, with no context (most important — describe what it actually looks like, not what you guess the intent to be)\n\
         - category_signal: whether it signals a fortune-telling/mysticism app\n\
         - legibility_29px: whether the silhouette holds up at small render sizes, or turns into a blur\n\
         - biggest_flaw: the most noticeable flaw (\"none\" if there isn't one)\n\n\
         Finally, list every candidate id from above in ranking, ordered from 1st place (no ties, no omissions).\n\n\
         ## Output (JSON only, no other text)\n\
         {{\"items\":[{{\"candidate_id\":\"...\",\"blind_read\":\"...\",\"category_signal\":\"...\",\
         \"legibility_29px\":\"...\",\"biggest_flaw\":\"...\"}}],\"ranking\":[\"...\"]}}",
        spec.context.trim(),
        spec.render_sizes.len(),
    )
}

/// A single critic's fully independent call. main.rs runs this N times in parallel via par_map,
/// assigning a different llm (different provider) per critic via spec.critic_backends to ensure jury diversity.
pub fn run_one(
    llm: &Llm,
    spec: &Spec,
    blind_ids: &[String],
    image_dir: &Path,
    critic_index: usize,
) -> Result<CritiqueRound> {
    // Cyclically shift the catalog order per critic — offsets position bias.
    let mut shifted = blind_ids.to_vec();
    if !shifted.is_empty() {
        let shift = critic_index % shifted.len();
        shifted.rotate_left(shift);
    }
    let prompt = round_prompt(spec, &shifted, image_dir);

    // The attached images must be in the exact same order as `shifted`. On the OpenRouter side,
    // images carry no filename metadata, so if the order drifts from the catalog order in the
    // prompt text, the model silently mispairs "first attached image = first id mentioned in the
    // text" (an actual bug we hit — every time critic_index=1 with shift=1, all labels ended up
    // shifted by one slot. The Claude CLI never had this problem, since it opens paths directly
    // with the Read tool).
    let mut images: Vec<PathBuf> = Vec::new();
    for id in &shifted {
        for &size in &spec.render_sizes {
            images.push(image_dir.join(format!("{id}_{size}.png")));
        }
    }

    let raw: RawResponse = llm
        .json_with_images(&prompt, Some(SYSTEM), &images)
        .with_context(|| {
            format!(
                "discourse critic {critic_index}({}) call failed",
                llm.provider_label
            )
        })?;

    let valid_ids: std::collections::HashSet<&String> = blind_ids.iter().collect();
    let ranking: Vec<String> = raw
        .ranking
        .into_iter()
        .filter(|id| valid_ids.contains(id))
        .collect();
    // Length alone doesn't guarantee "every candidate exactly once" — a ranking that duplicates
    // one candidate and omits another has the same length as a valid one and would silently slip
    // through, double-counting the duplicate and zeroing the omitted candidate's Borda score in
    // quantify.rs. Checking the count of distinct ids against blind_ids.len() as well closes that
    // gap: combined with the length check, it forces the filtered ranking to be exactly the set of
    // valid ids with no repeats and no gaps.
    let unique_ranked: std::collections::HashSet<&String> = ranking.iter().collect();
    anyhow::ensure!(
        ranking.len() == blind_ids.len() && unique_ranked.len() == blind_ids.len(),
        "critic {critic_index}({}) ranking does not cover all candidates exactly once ({} entries, {} unique, {} expected)",
        llm.provider_label,
        ranking.len(),
        unique_ranked.len(),
        blind_ids.len()
    );

    Ok(CritiqueRound {
        critic_index,
        provider: llm.provider_label.to_string(),
        items: raw.items,
        ranking,
    })
}
