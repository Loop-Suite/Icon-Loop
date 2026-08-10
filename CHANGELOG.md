# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-10

Initial release.

### Added

- `iconloop design` / `iconloop refine` / `iconloop validate` — the lens → render → policy →
  discourse → quantify pipeline: independent per-persona SVG generation, deterministic
  render/policy gates (containment, palette, legibility), fully independent blind critic discourse
  (mixed Claude CLI / OpenRouter backends), and deterministic Borda-count verdict aggregation.
- `evals/README.md` — empirical record of the round-1/round-2 static review findings (#2, #3, #4)
  and the real-execution checks that verified each fix, including two rollback reproductions of the
  original bugs.
- Dependabot configuration for automated dependency and GitHub Actions update PRs.

### Fixed

- CI `cargo fmt --check` step failing on `main` due to pre-existing, pre-rustfmt formatting (#1).
- OpenRouter critic image-count mismatch vs. catalog text: `round_prompt()`'s hardcoded "3 images"
  catalog text drifted from the actual number of images attached (one per `spec.render_sizes`
  entry), silently mispairing every candidate after the first on the OpenRouter backend, which has
  no filename metadata on attached images (#2).
- Ranking validation accepted duplicates+omissions with matching total length: a critic ranking
  that listed one candidate twice and omitted another had the same length as a valid ranking and
  passed unchanged, silently corrupting Borda aggregation and potentially flipping the winner (#3).
- `policy.rs`'s background match ignored alpha: `check_containment`/`check_legibility` compared
  only RGB against `background_hex`, so any canvas area an SVG never painted (left fully
  transparent by `Pixmap::new`'s zero-initialization) was misread as foreground for any
  `background_hex` other than black (#4).

### Security

- Path traversal / arbitrary file write: `render::render_all` built its PNG output path directly
  from an unsanitized `candidate_id` (sourced from `Persona.id` in the spec TOML, or from
  `state.json`'s `candidates[].id` on `iconloop validate`). A crafted id containing `..` could
  escape the intended output directory, and an absolute-path id was substituted for the entire base
  path by `PathBuf::join`, letting `save_png()` overwrite an arbitrary file the invoking user has
  write access to. Fixed by rejecting any candidate id containing a path separator or `..` before
  it reaches the render path (#13).
- Unbounded resource/cost exhaustion: `Spec::load` had no upper bound on `canvas`/`render_sizes`/
  `legibility_size` (a large-but-plausible value could trigger a multi-gigabyte `Pixmap` allocation
  that aborts the process) or on `n_critics`/`personas.len()` (each directly drives a real, billed
  LLM call with no cap, so a spec typo could turn one invocation into thousands of API calls). Fixed
  by adding sane upper bounds (8192px per render dimension, 50 max critics/personas) to
  `Spec::validate` (#14).
- Palette policy gate bypass: `check_palette`'s regex only matched double-quoted
  `fill="#rrggbb"` attributes, silently missing legally single-quoted `fill='#rrggbb'` — an
  off-palette color in a single-quoted attribute passed the deterministic palette gate undetected.
  Fixed by widening the regex to accept either quote character (#15).
- Latent API key exposure via `Debug`: `Provider`/`Llm` derived `Debug` on a variant holding the
  raw `OPENROUTER_API_KEY`, so any future `{:?}`-formatting (a stray `dbg!()`, an error wrapper, a
  log line) would have printed the key verbatim. No live leak was found in current call sites
  (`ureq::Error`'s `Display` never includes request headers), but closed as a latent footgun via a
  manual, redacting `Debug` impl (#16).

### Changed

- Dependency updates via Dependabot: `ureq` 2.12.1 → 3.3.0, `toml` 0.8.23 → 1.1.4+spec-1.1.0,
  `resvg`/`usvg` 0.47.0 → 0.48.1, `base64` 0.22.1 → 0.23.1, `clap` 4.6.5 → 4.6.6,
  `actions/checkout` 4 → 7.
