// Deterministic gate — no LLM involved. Mirrors codedesign-loop's policy.rs (ported the Pass/Fail + evidence pattern).
// Coordinate-based containment checks are computed directly from the actually rendered pixels (pixmap) — instead of
// regex-parsing the path string, this validates what usvg/resvg actually drew, so stroke and curve margins are accounted for.
use crate::models::{PolicyCheck, PolicyReport, PolicyStatus};
use crate::render::Rendered;
use crate::spec::Spec;
use anyhow::{Context, Result};
use regex::Regex;

const AA_TOLERANCE_PX: f32 = 3.0;

/// A pixel counts as "background" if its RGB matches `background_hex` exactly, or if it's fully
/// transparent. Pixmap::new zero-initializes its buffer, so any canvas area the SVG never actually
/// painted is left as premultiplied (0, 0, 0, 0) — and premultiplied alpha requires r,g,b <= a, so a
/// transparent pixel's RGB is always (0, 0, 0) no matter what background_hex says. Comparing RGB
/// alone would only coincidentally treat unpainted area as background when background_hex is
/// "#000000"; for any other background color, unpainted canvas (e.g. from a background rect that
/// doesn't fully cover the canvas) would be misread as foreground.
fn is_background(px: tiny_skia::PremultipliedColorU8, bg: (u8, u8, u8)) -> bool {
    px.alpha() == 0 || (px.red() == bg.0 && px.green() == bg.1 && px.blue() == bg.2)
}

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
        if is_background(*px, (bg_r, bg_g, bg_b)) {
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
    // Matches both `fill="#rrggbb"` and `fill='#rrggbb'` — XML attribute values may legally use
    // either quote character, and nothing constrains the lens LLM's output to double quotes
    // specifically, so a single-quote-only regex would silently let an off-palette color slip past
    // this gate whenever the LLM happens to emit single-quoted attributes. (The `regex` crate has
    // no backreferences, so this doesn't enforce that the opening/closing quote match — fine here,
    // since we're scanning for fill declarations, not validating XML well-formedness.)
    let re = Regex::new(r#"fill=["'](#[0-9a-fA-F]{6})["']"#).context("regex compilation failed")?;
    let allowed: std::collections::HashSet<String> =
        spec.palette.iter().map(|h| h.to_lowercase()).collect();

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
        status: if stray.is_empty() {
            PolicyStatus::Pass
        } else {
            PolicyStatus::Fail
        },
        evidence: if stray.is_empty() {
            format!(
                "All {} used colors are within the palette: {:?}",
                used.len(),
                used
            )
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
        .filter(|px| !is_background(**px, (bg_r, bg_g, bg_b)))
        .count() as f32;
    let ratio = fg / total;
    let ok = ratio >= spec.min_fg_ratio && ratio <= spec.max_fg_ratio;

    PolicyCheck {
        id: "legibility".to_string(),
        status: if ok {
            PolicyStatus::Pass
        } else {
            PolicyStatus::Fail
        },
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
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No render found for small legibility size ({})",
                spec.legibility_size
            )
        })?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_with_bg(background_hex: &str) -> Spec {
        Spec {
            name: "test".to_string(),
            context: String::new(),
            canvas: 100,
            margin_ratio: 0.10,
            render_sizes: vec![100],
            legibility_size: 100,
            min_fg_ratio: 0.0,
            // Deliberately below 1.0: before the alpha fix, the untouched (transparent) half of the
            // canvas was misread as foreground too, pushing the ratio to ~1.0 and failing this bound.
            max_fg_ratio: 0.6,
            background_hex: background_hex.to_string(),
            palette: vec!["#ffffff".to_string()],
            n_critics: 1,
            critic_backends: vec!["claude".to_string()],
            openrouter_critic_model: "x-ai/grok-4.5".to_string(),
            personas: vec![],
        }
    }

    // A background rect that only covers the top-left half of the canvas — simulating a
    // non-compliant LLM output that doesn't fully paint the canvas (rounded corners, a slightly
    // undersized rect, a viewBox mismatch, ...). The rest of the pixmap stays fully transparent.
    const HALF_COVERED_SVG: &str = r##"<svg viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg">
<rect x="0" y="0" width="100" height="50" fill="#ffffff"/>
</svg>"##;

    #[test]
    fn transparent_canvas_area_is_not_foreground_regardless_of_background_hex() {
        let tmp =
            std::env::temp_dir().join(format!("icon-loop-policy-test-{}", std::process::id()));
        let spec = spec_with_bg("#ff00ff"); // deliberately not black, unlike the shipped example spec
        let renders = crate::render::render_all(HALF_COVERED_SVG, &spec.render_sizes, &tmp, "c")
            .expect("render should succeed");
        let report =
            evaluate(&spec, "c", HALF_COVERED_SVG, &renders).expect("evaluate should succeed");
        let legibility = report
            .checks
            .iter()
            .find(|c| c.id == "legibility")
            .expect("legibility check present");
        // Only the painted top half (50%) should count as foreground — the untouched, fully
        // transparent bottom half must not be misread as foreground just because it doesn't match
        // background_hex's RGB.
        assert!(
            (report.fg_ratio - 0.5).abs() < 0.01,
            "expected ~50% foreground (only the painted half), got {}",
            report.fg_ratio
        );
        assert_eq!(legibility.status, PolicyStatus::Pass);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn unique_tmp_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "icon-loop-policy-test-{}-{label}",
            std::process::id()
        ))
    }

    #[test]
    fn palette_check_catches_stray_color_in_single_quoted_fill() {
        let spec = spec_with_bg("#000000");
        // Only #ffffff is in-palette; #ff00ff is a stray color written with single quotes — the
        // original double-quote-only regex would have missed this entirely and reported Pass.
        let svg =
            r#"<svg viewBox="0 0 100 100"><rect fill='#ff00ff' width="10" height="10"/></svg>"#;
        let check = check_palette(&spec, svg).expect("check_palette should succeed");
        assert_eq!(
            check.status,
            PolicyStatus::Fail,
            "a single-quoted stray fill color must be caught, not silently pass: {}",
            check.evidence
        );
    }

    #[test]
    fn palette_check_accepts_single_quoted_in_palette_fill() {
        let spec = spec_with_bg("#000000");
        let svg =
            r#"<svg viewBox="0 0 100 100"><rect fill='#ffffff' width="10" height="10"/></svg>"#;
        let check = check_palette(&spec, svg).expect("check_palette should succeed");
        assert_eq!(check.status, PolicyStatus::Pass, "{}", check.evidence);
    }

    #[test]
    fn translucent_foreground_pixel_is_counted_as_foreground_not_background() {
        // A half-opacity rect drawn over a black background, in a color that is NOT the background
        // color — the blended pixel's RGB will differ from background_hex, and its alpha is nonzero,
        // so it must count as foreground even though it's only half-opaque.
        let tmp = unique_tmp_dir("translucent");
        let spec = spec_with_bg("#000000");
        let svg = r##"<svg viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg">
<rect width="100" height="100" fill="#000000"/>
<rect x="0" y="0" width="100" height="100" fill="#ffffff" fill-opacity="0.5"/>
</svg>"##;
        let renders =
            crate::render::render_all(svg, &spec.render_sizes, &tmp, "c").expect("render");
        let report = evaluate(&spec, "c", svg, &renders).expect("evaluate");
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(
            report.fg_ratio > 0.9,
            "a translucent full-canvas overlay of a non-background color should read as ~100% foreground, got {}",
            report.fg_ratio
        );
    }

    #[test]
    fn gradient_alpha_yields_partial_foreground_ratio_without_panicking() {
        // A linear gradient fading opacity from 1 (left) to 0 (right) across the full canvas. This
        // isn't LLM-legal output (the lens prompt forbids gradients), but the renderer/policy code
        // must still handle a continuous range of per-pixel alpha values robustly — the alpha-aware
        // background check must not crash or degenerate on partial (non-binary) alpha.
        let tmp = unique_tmp_dir("gradient-alpha");
        let spec = spec_with_bg("#000000");
        let svg = r##"<svg viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg">
<defs>
<linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
<stop offset="0" stop-color="#ffffff" stop-opacity="1"/>
<stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
</linearGradient>
</defs>
<rect width="100" height="100" fill="#000000"/>
<rect x="0" y="0" width="100" height="100" fill="url(#g)"/>
</svg>"##;
        let renders =
            crate::render::render_all(svg, &spec.render_sizes, &tmp, "c").expect("render");
        let report = evaluate(&spec, "c", svg, &renders).expect("evaluate");
        let _ = std::fs::remove_dir_all(&tmp);
        // is_background() treats any nonzero alpha as foreground, so the fully-transparent right
        // edge aside, almost the entire gradient should read as foreground — this pins down current
        // behavior (nonzero alpha = foreground, regardless of how faint) rather than asserting a
        // specific fractional ratio, since the exact anti-aliased boundary is renderer-dependent.
        assert!(
            report.fg_ratio > 0.0,
            "gradient alpha must not evaluate to 0% foreground"
        );
        assert!(
            report.fg_ratio <= 1.0,
            "foreground ratio must not exceed 100%, got {}",
            report.fg_ratio
        );
    }
}
