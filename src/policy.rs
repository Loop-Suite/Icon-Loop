// Deterministic gate — no LLM involved. Mirrors codedesign-loop's policy.rs (ported the Pass/Fail + evidence pattern).
// Coordinate-based containment checks are computed directly from the actually rendered pixels (pixmap) — instead of
// regex-parsing the path string, this validates what usvg/resvg actually drew, so stroke and curve margins are accounted for.
use crate::models::{PolicyCheck, PolicyReport, PolicyStatus};
use crate::render::Rendered;
use crate::spec::Spec;
use anyhow::{Context, Result};
use regex::Regex;

const AA_TOLERANCE_PX: f32 = 3.0;

fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

fn check_containment(spec: &Spec, native_render: &Rendered) -> PolicyCheck {
    let (bg_r, bg_g, bg_b) = parse_hex(&spec.background_hex).unwrap_or((0, 0, 0));
    let pixmap = &native_render.pixmap;
    let (w, h) = (pixmap.width(), pixmap.height());
    let mut min_x = w as i64;
    let mut max_x = -1i64;
    let mut min_y = h as i64;
    let mut max_y = -1i64;

    for (i, px) in pixmap.pixels().iter().enumerate() {
        if px.red() == bg_r && px.green() == bg_g && px.blue() == bg_b {
            continue;
        }
        let x = (i as u32 % w) as i64;
        let y = (i as u32 / w) as i64;
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }

    if max_x < 0 {
        return PolicyCheck {
            id: "containment".to_string(),
            status: PolicyStatus::Fail,
            evidence: "No foreground pixels found — empty canvas".to_string(),
        };
    }

    let margin = spec.margin_px() - AA_TOLERANCE_PX;
    let hi = spec.canvas as f32 - margin;
    let ok = min_x as f32 >= margin
        && min_y as f32 >= margin
        && (max_x + 1) as f32 <= hi
        && (max_y + 1) as f32 <= hi;

    PolicyCheck {
        id: "containment".to_string(),
        status: if ok { PolicyStatus::Pass } else { PolicyStatus::Fail },
        evidence: format!(
            "bbox x[{min_x}..{max_x}] y[{min_y}..{max_y}] / allowed margin {margin:.0}px (canvas {})",
            spec.canvas
        ),
    }
}

fn check_palette(spec: &Spec, svg_source: &str) -> Result<PolicyCheck> {
    let re = Regex::new(r#"fill="(#[0-9a-fA-F]{6})""#).context("regex compilation failed")?;
    let allowed: std::collections::HashSet<String> = spec
        .palette
        .iter()
        .map(|h| h.to_lowercase())
        .collect();

    let mut used = std::collections::HashSet::new();
    let mut stray = Vec::new();
    for cap in re.captures_iter(svg_source) {
        let hex = cap[1].to_lowercase();
        used.insert(hex.clone());
        if !allowed.contains(&hex) && !stray.contains(&hex) {
            stray.push(hex);
        }
    }

    Ok(PolicyCheck {
        id: "palette".to_string(),
        status: if stray.is_empty() { PolicyStatus::Pass } else { PolicyStatus::Fail },
        evidence: if stray.is_empty() {
            format!("All {} used colors are within the palette: {:?}", used.len(), used)
        } else {
            format!("Colors used outside the palette: {:?}", stray)
        },
    })
}

fn check_legibility(spec: &Spec, legibility_render: &Rendered) -> PolicyCheck {
    let (bg_r, bg_g, bg_b) = parse_hex(&spec.background_hex).unwrap_or((0, 0, 0));
    let pixmap = &legibility_render.pixmap;
    let total = (pixmap.width() * pixmap.height()) as f32;
    let fg = pixmap
        .pixels()
        .iter()
        .filter(|px| !(px.red() == bg_r && px.green() == bg_g && px.blue() == bg_b))
        .count() as f32;
    let ratio = fg / total;
    let ok = ratio >= spec.min_fg_ratio && ratio <= spec.max_fg_ratio;

    PolicyCheck {
        id: "legibility".to_string(),
        status: if ok { PolicyStatus::Pass } else { PolicyStatus::Fail },
        evidence: format!(
            "{}px foreground ratio {:.1}% (allowed {:.0}%~{:.0}%)",
            legibility_render.size,
            ratio * 100.0,
            spec.min_fg_ratio * 100.0,
            spec.max_fg_ratio * 100.0
        ),
    }
}

pub fn evaluate(
    spec: &Spec,
    candidate_id: &str,
    svg_source: &str,
    renders: &[Rendered],
) -> Result<PolicyReport> {
    let native = renders
        .iter()
        .find(|r| r.size == spec.canvas)
        .ok_or_else(|| anyhow::anyhow!("No render found for canvas size ({})", spec.canvas))?;
    let legibility = renders
        .iter()
        .find(|r| r.size == spec.legibility_size)
        .ok_or_else(|| anyhow::anyhow!("No render found for small legibility size ({})", spec.legibility_size))?;

    let containment = check_containment(spec, native);
    let palette = check_palette(spec, svg_source)?;
    let legibility_check = check_legibility(spec, legibility);
    let fg_ratio = {
        let evidence = &legibility_check.evidence;
        evidence
            .split_whitespace()
            .find_map(|tok| tok.strip_suffix('%').and_then(|v| v.parse::<f32>().ok()))
            .unwrap_or(0.0)
            / 100.0
    };

    let checks = vec![containment, palette, legibility_check];
    let overall_pass = checks.iter().all(|c| c.status == PolicyStatus::Pass);

    Ok(PolicyReport {
        candidate_id: candidate_id.to_string(),
        checks,
        fg_ratio,
        overall_pass,
        render_paths: renders
            .iter()
            .map(|r| (r.size, r.path.to_string_lossy().to_string()))
            .collect(),
    })
}
