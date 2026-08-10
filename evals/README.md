# Empirical review findings — icon-loop

This directory records what actually happened during review passes on this repo: static review
findings that became issues and fixes, a pass that ran the real CLI (`claude -p` + OpenRouter, real
API cost where noted) to check whether the fixes actually held up — including, for two of the
first three findings, rolling back to the pre-fix code and re-running the *same* input to watch
the original bug reproduce — and later a separate adversarial re-audit (round 3) that found and
fixed four more issues, including a path-traversal arbitrary-file-write, before the `v0.1.0` tag.
This file is the record of that, not a promotional summary.

## TL;DR

| Stage | Method | Result |
|---|---|---|
| Static review, round 1 | Manual code read, no execution | 2 real bugs found ([#2](https://github.com/Loop-Suite/Icon-Loop/issues/2), [#3](https://github.com/Loop-Suite/Icon-Loop/issues/3)), both fixed |
| Static review, round 2 (deeper pass) | Manual code read, no execution | 1 more real bug found ([#4](https://github.com/Loop-Suite/Icon-Loop/issues/4)), fixed |
| #2 fix check | Real `claude -p` + OpenRouter run, `render_sizes` deliberately set to 2 (not the default 3) | Critic received and ranked the correct image count/order |
| #3 fix check | Local fake-`claude` binary (zero API cost), duplicate+omission ranking injected | Post-fix: rejected immediately. Pre-fix (rolled back): passed silently, wrong winner selected |
| #4 fix check | `iconloop validate` (deterministic render, no LLM calls), non-black background + half-painted SVG | Post-fix: transparent area correctly excluded. Pre-fix (rolled back): containment FAIL, bbox inflated to full canvas |
| End-to-end re-run | Real `claude -p` + OpenRouter with user-supplied API key, mixed critic backends, 2 personas, non-black background spec | Policy gate PASS, both critics responded, winner selected, `report.md` generated |
| Static review, round 3 (adversarial re-audit) | Manual code read, no execution | 4 more real bugs found ([#13](https://github.com/Loop-Suite/Icon-Loop/issues/13) path traversal, [#14](https://github.com/Loop-Suite/Icon-Loop/issues/14) resource exhaustion, [#15](https://github.com/Loop-Suite/Icon-Loop/issues/15) palette gate bypass, [#16](https://github.com/Loop-Suite/Icon-Loop/issues/16) latent key-leak footgun), all fixed; SVG entity bomb/XXE audited and ruled out (already mitigated upstream by `roxmltree`) |
| Edge-case test suite | `cargo test` | 1 → 26 tests (malformed/truncated/empty SVG, empty spec file, canvas/render bounds at 0/1/8192/50000, translucent pixels, gradient alpha) |
| Versioning | `CHANGELOG.md` + git tag | `v0.1.0` tagged and released |
| Local validate spot-check | `iconloop validate` (no LLM calls, $0) | Translucent + gradient-alpha specs: no crash/panic on either; behavior matches the round-3 regression tests |

No further bugs were found once real execution started — everything from that point on was
verification that the static-review fixes actually work, not new discovery (round 3's adversarial
re-audit is a separate, later static-review pass — see below).

**What this bought:**

- **Static review found 3 real, fixable bugs before spending a single dollar on a live model
  call.** All three ([#2](https://github.com/Loop-Suite/Icon-Loop/issues/2),
  [#3](https://github.com/Loop-Suite/Icon-Loop/issues/3),
  [#4](https://github.com/Loop-Suite/Icon-Loop/issues/4)) were confirmed by reading the code, not
  by seeing something fail — cheaper than finding the same bugs via a live run that happened to
  hit the right input.
- **Two of the three fixes were verified by reproducing the original bug, not just by reading the
  diff.** For #3 and #4, the pre-fix commit was checked out, the exact same adversarial input was
  re-run, and the original failure reproduced live — then the tree was restored to the fix. This
  is a stronger check than "the code review argument sounds right": it's "the unfixed code
  actually fails this input, and the fixed code doesn't," observed directly rather than inferred.
- **The #3 rollback reproduction cost nothing** — a local fake `claude` binary stands in for the
  real CLI, so the duplicate/omission ranking exploit and its silent-wrong-winner consequence were
  reproduced with $0 in API spend. **The #4 rollback reproduction also cost nothing** — `iconloop
  validate` runs the render/policy pipeline deterministically with no LLM call involved at all.
  Neither of these needed to spend real money to get a real reproduction.
- **#2's fix was checked against a case the default config never exercises.** The default spec
  uses `render_sizes` with 3 entries, which is also what the old hardcoded prompt text assumed —
  so a fix that happened to only work at exactly 3 would have looked identical to a correct fix
  under the default config. Setting `render_sizes` to 2 for the live check forces the catalog text
  and the actual attached image count to only agree if the fix generalizes, not coincidentally
  matches the default.
- **One end-to-end run with a real user-supplied OpenRouter key** confirmed the full pipeline
  (lens → render → policy → discourse → quantify) still produces a complete, correct result
  (policy PASS, both critics responding, a winner selected, `report.md` written) with a
  non-default, non-black background color — the exact condition #4's bug depended on to be
  invisible.
- **Honest limitation, stated plainly:** icon-loop does not log its own LLM call count or cost
  (there's no equivalent of Code-Review-Loop's `manifest.json`/`usage.calls`), so unlike that
  project's evals, the numbers here cannot report an exact real dollar cost per run. Costs below
  are rough estimates from call count and known per-call model pricing, marked as such, not
  measured totals.
- **A later adversarial re-audit (round 3, below) found the most serious bug in this repo's
  history before `v0.1.0` shipped it**: [#13](https://github.com/Loop-Suite/Icon-Loop/issues/13),
  a path traversal / arbitrary file write via an unsanitized candidate id. Static review caught it
  the same way it caught #2–#4 — by reading the code, not by triggering it — and it was fixed,
  tested, and tagged into the same release rather than discovered after.

## Round 1: static review — #2, #3

Both found by reading `src/discourse.rs` before any code was executed.

**[#2](https://github.com/Loop-Suite/Icon-Loop/issues/2) — OpenRouter critic image-count mismatch
vs. catalog text.** `round_prompt()` told the critic "Check 3 images per candidate (large size /
60px / {legibility_size}px)" as a hardcoded string, while `run_one()` actually attaches one image
per `spec.render_sizes` entry (4 by default, not 3). The OpenRouter backend has no filename
metadata on attached images — it has to infer where one candidate's images end and the next
one's begin purely from the stated count. A prompt claiming the wrong count silently mispairs
every candidate after the first, corrupting the critic's read without any error or warning.

**[#3](https://github.com/Loop-Suite/Icon-Loop/issues/3) — ranking validation accepts
duplicates+omissions with matching total length.** `run_one()`'s ranking check was
`ranking.len() == blind_ids.len()` only. A ranking that lists one candidate twice and omits
another has the same length as a valid one and passed unchanged — silently double-counting the
duplicate and zero-counting the omitted candidate in `quantify.rs`'s Borda aggregation, which can
flip the winner without any visible error.

Both fixed in a single commit
([`850f14b`](https://github.com/Loop-Suite/Icon-Loop/commit/850f14bb2e7713092ea7566f7cbaa3c5b86e9c80)):
the catalog text and image count are now both derived from `spec.render_sizes` so they can't drift
apart, and the ranking check now also compares the count of *distinct* ids
(`HashSet<&String>`) against `blind_ids.len()`, which — combined with the existing length check —
forces the ranking to be exactly the valid id set with no repeats and no gaps.

## Round 2: deeper static review — #4

A second, more careful pass over `src/policy.rs` (the deterministic geometry/color gate, run
before any LLM critic sees a candidate) found:

**[#4](https://github.com/Loop-Suite/Icon-Loop/issues/4) — background match ignores alpha:
transparent canvas area misread as foreground unless `background_hex` is black.**
`check_containment` and `check_legibility` compared only RGB against `background_hex`, never
alpha. `tiny_skia::Pixmap::new` zero-initializes its buffer, so any canvas area an SVG never
actually paints stays premultiplied `(0,0,0,0)` — and premultiplied alpha requires `r,g,b <= a`,
so a fully transparent pixel's RGB channels are always `(0,0,0)`, regardless of the configured
background color. The RGB-only comparison only *coincidentally* treated unpainted area as
background when `background_hex == "#000000"` (the value the shipped example spec happens to
use) — for any other background color, unpainted canvas silently counted as foreground, which is
exactly the class of non-compliant-SVG defect this deterministic gate exists to catch.

Fixed in
[`7e4c8ed`](https://github.com/Loop-Suite/Icon-Loop/commit/7e4c8ededa108e3fb0c10815d520f4fe3e0e1abb):
adds `is_background()`, which treats `alpha == 0` as background regardless of RGB, plus a
regression test
(`transparent_canvas_area_is_not_foreground_regardless_of_background_hex`) that reproduces the bug
with a half-painted canvas and a non-black `background_hex`.

## Real execution: checking the fixes, not just reading them

Static review can be wrong about its own reasoning. The next step ran the actual CLI against the
fixed code, then — for the two findings where it was feasible to do cheaply — rolled back to the
pre-fix commit and re-ran the identical input to confirm the original bug was real and is what the
fix actually addresses. **No new bugs were found in this stage.** Every result below is a
confirmation of a fix already merged, not a new discovery.

### #2 — real OpenRouter critic call with a non-default image count

Ran `iconloop design` with `render_sizes` deliberately set to 2 entries instead of the default 3,
so the check couldn't pass by coincidentally matching whatever number the old hardcoded prompt
text used to assume. This used a real `claude -p` call for the lens stage and a real OpenRouter
critic call (paid, see cost note below). Result: the critic's response correctly reflected 2
images per candidate, in the derived order — the catalog text (`sizes_label`) and the actual
attached image count agreed, confirming the fix generalizes rather than only working at the
default size count.

No rollback comparison was run for #2 — reproducing the original mispairing bug live would require
a critic response that happens to visibly reveal a candidate mix-up, which isn't a reliable
observation to force on demand the way #3 and #4's deterministic checks are. The fix was checked
by confirming correct behavior under a condition (`render_sizes != 3`) the old code couldn't have
satisfied by accident, which is weaker evidence than a rollback reproduction — noted honestly here
rather than overstated.

### #3 — fake-`claude` binary, rollback reproduction, zero cost

Used a local fake `claude` binary (returns a scripted response instead of calling any real model)
to inject an adversarial ranking that lists one blind candidate id twice and omits another — the
exact shape #3's fix targets. This has zero API cost since no real model is called.

- **Post-fix (current code):** rejected immediately with the new "does not cover all candidates
  exactly once" error, before the ranking ever reached `quantify.rs`.
- **Rolled back to the pre-fix commit, same adversarial input re-run:** passed the length-only
  check silently — no error — and a wrong winner was selected by Borda aggregation, exactly as
  #3 described (the duplicate double-counted, the omitted candidate zero-counted).
- Tree restored to the fix afterward (`git checkout` back to HEAD on the fix commit; no changes
  left behind).

This is the strongest verification in this document: not "the fixed code looks correct," but
"the exact same adversarial ranking was fed to both the unfixed and fixed code, and only the
unfixed version produced the wrong, silent outcome the issue predicted."

### #4 — `iconloop validate`, rollback reproduction, zero cost

Built a spec with a non-black background (`#1F2D3D`) and an SVG that only paints half the canvas,
then ran `iconloop validate` — the deterministic render/policy path, no LLM call at all, so also
zero cost.

- **Post-fix (current code):** the transparent (unpainted) half of the canvas was correctly
  excluded from the foreground bounding box; containment passed as expected.
- **Rolled back to the pre-fix commit, same spec/SVG re-run:** containment **FAILED**, with the
  reported foreground bounding box inflated to the full canvas — the unpainted, fully transparent
  half was misread as foreground, exactly as #4 described, and exactly reproducing the
  RGB-only-vs-alpha-aware distinction the fix makes.
- Tree restored to the fix afterward, same as #3.

Like #3, this is a direct reproduction: the same non-black-background, half-painted input was run
through both the pre-fix and post-fix policy gate, and only the pre-fix version produced the
predicted failure.

### End-to-end re-run with a real user API key

Separately, ran `iconloop design` with the user's own real OpenRouter API key, mixing a `claude -p`
lens/critic with an OpenRouter critic, 2 personas, and a spec using a non-black background color
(the exact condition #4's bug needed to be invisible under the old code). Result: policy gate
PASS, both critics returned valid responses, a winner was selected, and `report.md` was generated
correctly end to end. This wasn't a targeted regression check for any single issue — it was a
general confidence check that the fixed pipeline still produces a complete, working result under
close-to-real usage conditions (real key, mixed backends, non-default spec).

## Round 3: production-hardening — adversarial re-audit, edge cases, versioning

A later, separate pass, not a continuation of rounds 1–2: instead of reading for correctness bugs,
this pass re-read `policy.rs`, `render.rs`, `spec.rs`, and `llm.rs` adversarially — "what does an
attacker-controlled or merely malformed input make this code do" — plus added the edge-case test
coverage that was still missing, and cut the first tagged release. All landed in
[PR #17](https://github.com/Loop-Suite/Icon-Loop/pull/17),
[PR #18](https://github.com/Loop-Suite/Icon-Loop/pull/18), and
[PR #19](https://github.com/Loop-Suite/Icon-Loop/pull/19).

### Adversarial re-audit — #13, #14, #15, #16

**[#13](https://github.com/Loop-Suite/Icon-Loop/issues/13) — path traversal / arbitrary file write
via unsanitized candidate id (most severe finding in this document).** `render::render_all` built
its PNG output path with `out_dir.join(format!("{candidate_id}_{size}.png"))`, and `candidate_id`
flows in unsanitized from `Persona.id` in the spec TOML (`design`/`refine`) or from
`state.json`'s `candidates[].id` (`validate`) — both are attacker/author-controlled data, not
generated internally. A `candidate_id` containing `..` can walk out of `out_dir`; an absolute-path
`candidate_id` is worse, because `PathBuf::join` silently discards the base path entirely and
resolves to the absolute path as-is. Either way, `save_png()` becomes an arbitrary file
write/overwrite, gated only by the invoking OS user's file permissions. Fixed by adding
`ensure_safe_id()` (rejects empty ids, ids containing `/` or `\`, and ids equal to `.` or `..`),
called at the top of `render_all` so both entry points are covered by one check
([`806de3a`](https://github.com/Loop-Suite/Icon-Loop/commit/806de3a4e618e4f81a2a347ee74729265e7dd065)).

**[#14](https://github.com/Loop-Suite/Icon-Loop/issues/14) — no upper bound on
canvas/render_sizes/n_critics/personas: resource exhaustion.** `Spec::load` only checked lower
bounds (non-empty personas/palette, `n_critics >= 1`); nothing capped the size fields or the count
fields. Two distinct failure modes: (1) `render::render_all` allocates a `width*height*4`-byte
`Pixmap` per render size with no other cap — a size well under `tiny_skia`'s own `i32::MAX/4`
limit (e.g. 50,000px) is already a multi-gigabyte allocation, and Rust's default allocator aborts
the process on allocation failure rather than returning a recoverable error, so a bad spec value
crashes the whole process, not just that render; (2) `n_critics` and `personas.len()` directly
control how many real, billed LLM calls one invocation makes, with no confirmation step, so a
typo'd spec could turn one `iconloop design` into thousands of API calls. Fixed by adding
`Spec::validate` with `MAX_RENDER_DIMENSION = 8192` (px, per canvas/render_sizes/legibility_size
dimension) and `MAX_CALL_COUNT = 50` (for `n_critics` and `personas.len()`) — generous headroom
above the shipped example spec (1024px canvas, 3 personas, `n_critics=3`), called from `load()`.

**[#15](https://github.com/Loop-Suite/Icon-Loop/issues/15) — palette policy gate bypassed by
single-quoted `fill` attributes.** `check_palette`'s regex was
`fill="(#[0-9a-fA-F]{6})"` — double-quoted only. XML attribute values may legally use either quote
character (`fill='#rrggbb'` is just as valid as `fill="#rrggbb"`), and nothing in the lens prompt
(`src/lens.rs`) constrains the LLM's SVG output to double-quoted attributes specifically — it only
constrains which hex values are allowed. A single-quoted, off-palette fill color was therefore
invisible to the regex and passed the deterministic palette gate undetected, defeating the
guarantee that gate exists to provide. Fixed by widening the regex to
`fill=["'](#[0-9a-fA-F]{6})["']` (the `regex` crate has no backreference support, so this doesn't
require the opening/closing quote to match each other — acceptable here since the check is
scanning for fill declarations, not validating XML well-formedness).

**[#16](https://github.com/Loop-Suite/Icon-Loop/issues/16) — `Provider`'s derived `Debug` would
print the raw `OPENROUTER_API_KEY` verbatim if ever `{:?}`-formatted.** `Provider::OpenRouter {
api_key: String }` derived `Debug`, so a future stray `dbg!()`, an error wrapper that formats with
`{:?}` instead of `{}`, or a new log line would print the key in plain text. Checked every current
call site before fixing: `ureq::Error`'s `Display` impl (verified against the vendored `ureq`
3.3.0 source, not assumed) never includes request headers, and every `call_openrouter` error
branch only formats the response body, never the outgoing `Authorization` header — so **no live
leak exists in the code as shipped today**. Fixed anyway, as a latent footgun independent of
whether any current call site exercises it: replaced the derive with a manual `Debug` impl on
`Provider` that redacts `api_key` to `"***REDACTED***"`; `Llm` keeps its derive since it no longer
exposes the raw key once `Provider`'s `Debug` is fixed.

**Audited and ruled out (no fix needed):** SVG entity-expansion ("billion laughs") and XXE.
`usvg`'s underlying `roxmltree` parser has its own built-in entity-reference-loop/depth limit
(≤10 nesting levels, ≤255 total references) and does not resolve external entities by default
(`entity_resolver: None`) — both classes of attack are already mitigated upstream, independent of
anything in this repo's own code.

### Edge-case test suite: 1 → 26 tests

Before this round the repo had exactly 1 test. [PR #17](https://github.com/Loop-Suite/Icon-Loop/pull/17)
added regression tests for all four fixes above plus general malformed/empty-input coverage
(malformed SVG, truncated SVG, empty SVG, empty spec file, a zero render size) — 21 tests total.
[PR #18](https://github.com/Loop-Suite/Icon-Loop/pull/18) closed the remaining gap: round-1
testing covered the *upper* size bound (oversized canvas/render_sizes from #14) but not the
*lower*/zero boundary. Added `canvas=0`, `legibility_size=0`, and a zero `render_sizes` entry all
rejected by `Spec::validate`; `canvas=1` (smallest legal value) accepted and actually renders a
correct 1×1 `Pixmap` — the positive counterpart to the existing "size=0 is an error" test. Final
count: 26 tests, all passing under `cargo test` / `cargo clippy --all-targets -- -D warnings` /
`cargo fmt --check`.

### Versioning: CHANGELOG.md + v0.1.0

[PR #19](https://github.com/Loop-Suite/Icon-Loop/pull/19) added `CHANGELOG.md` (Keep a Changelog
format) covering the full history — initial pipeline, the three round-1/round-2 fixes (#2–#4), the
four round-3 security/robustness fixes (#13–#16), and dependency bumps. `Cargo.toml` was already
at `version = "0.1.0"`, so no version bump was needed. Tagged and released as
[`v0.1.0`](https://github.com/Loop-Suite/Icon-Loop/releases/tag/v0.1.0).

### Local `iconloop validate` spot-check: translucent + gradient alpha

A final manual check, separate from the `cargo test` suite: ran `iconloop validate` locally (the
deterministic render/policy path — no LLM call, $0 cost) against two specs already covered by
round-3's regression tests, to confirm the CLI itself behaves correctly end-to-end and not just at
the unit-test level.

- **Translucent-fill spec:** `check_containment` **FAIL**, `check_palette` **PASS**,
  `check_legibility` **PASS** (foreground ratio 100%). The containment FAIL is expected, not a
  regression — the spec used is a deliberate full-canvas SVG, so containment correctly rejects it
  for painting the entire canvas rather than fitting within the intended bounds.
- **Gradient-alpha spec:** same three results (containment FAIL / palette PASS / legibility PASS),
  for the same reason.
- Both runs exited cleanly — no crash, no panic. Both confirm the current, intentional behavior
  documented by the round-3 regression tests: `is_background()` only treats `alpha == 0` as
  background, so a gradient's near-zero-but-nonzero alpha at its edge is still counted as
  foreground, not background. This is the same alpha-handling boundary the
  `gradient_alpha_yields_partial_foreground_ratio_without_panicking` test pins down — the local CLI
  run reproduces the same behavior outside the test harness, at $0 additional cost.

## Cost note (read before trusting any dollar figure here)

icon-loop does not log its own LLM call count or spend anywhere in its output — there is no
equivalent of Code-Review-Loop's `manifest.json`/`usage.calls` marker to read back after a run.
Every cost figure in this document is therefore an estimate from known call count and public
per-call model pricing, not a measured total:

- The #2 fix check and the end-to-end re-run each involved real `claude -p` calls (a Haiku-tier
  lens call and a Sonnet-tier critic call with attached images) plus one real OpenRouter critic
  call. Rough estimate for the first review round's real calls: **$0.2–$0.4, not precisely known**
  — flagged here as an estimate, not a fact, specifically because the tool gives no way to confirm
  it.
- The user's own-API-key end-to-end re-run involved additional real calls; exact cost is likewise
  unknown for the same reason.
- The #3 and #4 rollback reproductions cost **$0** — a fake `claude` binary and a no-LLM
  deterministic validator, respectively — which is also why those two checks are the ones with the
  most confidence behind them: cheap enough to run twice (pre-fix and post-fix) without cost being
  a reason not to.
- **Round 3 (the #13–#16 adversarial re-audit, the 1→26 edge-case tests, and the translucent/
  gradient `iconloop validate` spot-check) cost $0 in total.** All of it was static code review,
  `cargo test`, or the no-LLM `validate` path — no `claude -p` or OpenRouter calls anywhere in this
  round, unlike round 1's #2 check and the end-to-end re-run above.

If this repo's own CLI is extended to log call counts/cost the way Code-Review-Loop's
`manifest.json` does, a natural follow-up is re-running this same set of checks with exact numbers
instead of estimates.
