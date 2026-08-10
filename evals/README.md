# Empirical review findings — icon-loop

This directory records what actually happened during a review pass on this repo: static review
findings that became issues and fixes, then a second pass that ran the real CLI (`claude -p` +
OpenRouter, real API cost where noted) to check whether the fixes actually held up — including,
for two of the three findings, rolling back to the pre-fix code and re-running the *same* input to
watch the original bug reproduce. This file is the record of that, not a promotional summary.

## TL;DR

| Stage | Method | Result |
|---|---|---|
| Static review, round 1 | Manual code read, no execution | 2 real bugs found ([#2](https://github.com/Loop-Suite/Icon-Loop/issues/2), [#3](https://github.com/Loop-Suite/Icon-Loop/issues/3)), both fixed |
| Static review, round 2 (deeper pass) | Manual code read, no execution | 1 more real bug found ([#4](https://github.com/Loop-Suite/Icon-Loop/issues/4)), fixed |
| #2 fix check | Real `claude -p` + OpenRouter run, `render_sizes` deliberately set to 2 (not the default 3) | Critic received and ranked the correct image count/order |
| #3 fix check | Local fake-`claude` binary (zero API cost), duplicate+omission ranking injected | Post-fix: rejected immediately. Pre-fix (rolled back): passed silently, wrong winner selected |
| #4 fix check | `iconloop validate` (deterministic render, no LLM calls), non-black background + half-painted SVG | Post-fix: transparent area correctly excluded. Pre-fix (rolled back): containment FAIL, bbox inflated to full canvas |
| End-to-end re-run | Real `claude -p` + OpenRouter with user-supplied API key, mixed critic backends, 2 personas, non-black background spec | Policy gate PASS, both critics responded, winner selected, `report.md` generated |

No further bugs were found once real execution started — everything from that point on was
verification that the three static-review fixes actually work, not new discovery.

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

If this repo's own CLI is extended to log call counts/cost the way Code-Review-Loop's
`manifest.json` does, a natural follow-up is re-running this same set of checks with exact numbers
instead of estimates.
