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

pub fn render_all(
    svg: &str,
    sizes: &[u32],
    out_dir: &Path,
    candidate_id: &str,
) -> Result<Vec<Rendered>> {
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
