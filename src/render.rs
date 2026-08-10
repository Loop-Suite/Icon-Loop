// Renderer based on resvg/usvg/tiny-skia (all open source, by RazrFalcon).
// SVG -> PNG in pure Rust, with no dependency on external binary subprocesses (e.g. rsvg-convert).
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tiny_skia::{Pixmap, Transform};

pub struct Rendered {
    pub size: u32,
    pub path: PathBuf,
    pub pixmap: Pixmap,
}

/// `candidate_id` ends up as a filename component (`{candidate_id}_{size}.png`) joined onto
/// `out_dir`. It originates from `Persona.id` in the spec TOML (design/refine) or from
/// `state.json`'s `candidates[].id` (validate) — neither is sanitized upstream, and `PathBuf::join`
/// does not neutralize `..` segments or reject absolute-path components (an absolute component
/// silently replaces the whole base path). Without this check, a crafted id like
/// `"../../../../tmp/pwned"` or `"/Users/x/.ssh/authorized_keys"` turns a render call into an
/// arbitrary file write outside `out_dir`.
fn ensure_safe_id(candidate_id: &str) -> Result<()> {
    anyhow::ensure!(
        !candidate_id.is_empty()
            && !candidate_id.contains('/')
            && !candidate_id.contains('\\')
            && candidate_id != "."
            && candidate_id != "..",
        "unsafe candidate id (must not be empty or contain path separators): {candidate_id:?}"
    );
    Ok(())
}

pub fn render_all(
    svg: &str,
    sizes: &[u32],
    out_dir: &Path,
    candidate_id: &str,
) -> Result<Vec<Rendered>> {
    ensure_safe_id(candidate_id)?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &opt).context("SVG parsing failed (usvg)")?;
    let native = tree.size();
    std::fs::create_dir_all(out_dir).with_context(|| {
        format!(
            "failed to create render output directory: {}",
            out_dir.display()
        )
    })?;

    let mut out = Vec::new();
    for &size in sizes {
        let mut pixmap = Pixmap::new(size, size)
            .ok_or_else(|| anyhow!("Pixmap creation failed (size={size})"))?;
        let scale = size as f32 / native.width().max(1.0);
        let transform = Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        let path = out_dir.join(format!("{candidate_id}_{size}.png"));
        pixmap
            .save_png(&path)
            .with_context(|| format!("failed to save PNG: {}", path.display()))?;
        out.push(Rendered { size, path, pixmap });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_SVG: &str = r##"<svg viewBox="0 0 10 10" xmlns="http://www.w3.org/2000/svg">
<rect width="10" height="10" fill="#000000"/>
</svg>"##;

    fn unique_tmp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "icon-loop-render-test-{}-{label}",
            std::process::id()
        ))
    }

    #[test]
    fn rejects_parent_dir_traversal_in_candidate_id() {
        let tmp = unique_tmp_dir("traversal");
        let result = render_all(MINIMAL_SVG, &[10], &tmp, "../../../../tmp/pwned");
        assert!(
            result.is_err(),
            "a candidate id containing '..' must be rejected, not silently escape out_dir"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_absolute_path_candidate_id() {
        let tmp = unique_tmp_dir("absolute");
        // PathBuf::join replaces the base entirely when given an absolute component — without the
        // guard in render_all, this id would make save_png() write straight to that absolute path.
        let result = render_all(MINIMAL_SVG, &[10], &tmp, "/tmp/icon-loop-should-not-exist");
        assert!(
            result.is_err(),
            "an absolute-path candidate id must be rejected"
        );
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file("/tmp/icon-loop-should-not-exist_10.png");
    }

    #[test]
    fn rejects_empty_candidate_id() {
        let tmp = unique_tmp_dir("empty-id");
        let result = render_all(MINIMAL_SVG, &[10], &tmp, "");
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn malformed_svg_returns_error_not_panic() {
        let tmp = unique_tmp_dir("malformed");
        let result = render_all("this is not xml at all", &[10], &tmp, "c");
        assert!(
            result.is_err(),
            "malformed SVG must return an Err, not panic"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn truncated_unclosed_svg_returns_error_not_panic() {
        let tmp = unique_tmp_dir("truncated");
        let result = render_all(
            r#"<svg viewBox="0 0 10 10"><rect width="10" height="10"#,
            &[10],
            &tmp,
            "c",
        );
        assert!(
            result.is_err(),
            "truncated SVG must return an Err, not panic"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn empty_svg_string_returns_error_not_panic() {
        let tmp = unique_tmp_dir("empty-svg");
        let result = render_all("", &[10], &tmp, "c");
        assert!(
            result.is_err(),
            "an empty SVG string must return an Err, not panic"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn zero_render_size_returns_error_not_panic() {
        let tmp = unique_tmp_dir("zero-size");
        // Pixmap::new(0, 0) returns None ("Zero size is an error") — render_all must surface that
        // as a normal Err rather than unwrapping/panicking.
        let result = render_all(MINIMAL_SVG, &[0], &tmp, "c");
        assert!(
            result.is_err(),
            "a zero render size must return an Err, not panic"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
