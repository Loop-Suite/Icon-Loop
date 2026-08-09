use crate::llm::Llm;
use crate::models::IconCandidate;
use crate::spec::{Persona, Spec};
use anyhow::{Context, Result};

fn build_prompt(
    spec: &Spec,
    persona: &Persona,
    dead_concepts: &str,
    prior: Option<(&str, &str)>,
) -> String {
    let palette_list = spec.palette.join(", ");
    let refine_block = match prior {
        Some((prior_svg, critique_text)) => format!(
            "\n## Your Previous Round Result and Received Critiques\nPrevious SVG:\n```\n{prior_svg}\n```\n\
             Critiques left by the blind critics (verbatim):\n{critique_text}\n\n\
             Actually incorporate the above critiques to improve the design. You may rebuild it entirely or refine the existing structure.\n",
        ),
        None => String::new(),
    };

    format!(
        "You are an icon designer persona named \"{persona_name}\".\n\
         Design philosophy: {philosophy}\n\n\
         ## Product Context\n{context}\n\n\
         ## Fixed Constraints\n\
         - Background {bg}, viewBox=\"0 0 {canvas} {canvas}\"\n\
         - All shapes must be fully contained within {margin_pct:.0}% margin from each edge of the canvas (strictly enforce coordinate bounds, calculate and verify by hand)\n\
         - Use only the following hex values for fill: {palette}\n\
         - Use only <path>/<polygon>/<circle>; no <defs>/gradients/filters/text elements\n\
         - Flat vector only; no 3D shading/shadows/blur\n\n\
         ## Dead Concepts (do not repeat)\n{dead_concepts}\n\
         {refine_block}\n\
         ## Output\nOutput only the SVG code with no other explanation. It must start with `<svg viewBox=\"0 0 {canvas} {canvas}\" xmlns=\"http://www.w3.org/2000/svg\">` and end with `</svg>`. \
         It must include `<rect width=\"{canvas}\" height=\"{canvas}\" fill=\"{bg}\"/>` as the background, as the first shape.",
        persona_name = persona.persona_name,
        philosophy = persona.philosophy,
        context = spec.context.trim(),
        bg = spec.background_hex,
        canvas = spec.canvas,
        margin_pct = spec.margin_ratio * 100.0,
        palette = palette_list,
        dead_concepts = if dead_concepts.trim().is_empty() { "(none — round 1)" } else { dead_concepts },
    )
}

fn extract_svg(raw: &str) -> Result<String> {
    let start = raw.find("<svg").ok_or_else(|| {
        anyhow::anyhow!(
            "No <svg> found in response: {}",
            crate::llm::truncate(raw, 200)
        )
    })?;
    let end = raw
        .rfind("</svg>")
        .map(|i| i + "</svg>".len())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No </svg> found in response: {}",
                crate::llm::truncate(raw, 200)
            )
        })?;
    anyhow::ensure!(start < end, "Invalid SVG tag range");
    Ok(raw[start..end].to_string())
}

pub fn run(
    llm: &Llm,
    spec: &Spec,
    persona: &Persona,
    dead_concepts: &str,
    prior: Option<(&str, &str)>,
) -> Result<IconCandidate> {
    let prompt = build_prompt(spec, persona, dead_concepts, prior);
    let raw = llm
        .text(&prompt, None)
        .with_context(|| format!("lens call failed: {}", persona.id))?;
    let svg = extract_svg(&raw)?;
    Ok(IconCandidate {
        id: persona.id.clone(),
        persona_id: persona.id.clone(),
        persona_name: persona.persona_name.clone(),
        svg,
        rationale: String::new(),
    })
}
