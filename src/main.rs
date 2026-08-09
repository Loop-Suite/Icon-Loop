mod discourse;
mod lens;
mod llm;
mod models;
mod policy;
mod quantify;
mod render;
mod report;
mod spec;
mod state;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use llm::Llm;
use spec::{Persona, Spec};
use std::path::{Path, PathBuf};

#[derive(clap::ValueEnum, Clone, Debug, PartialEq)]
enum Backend {
    Claude,
    Openrouter,
}

#[derive(Parser, Debug)]
#[command(
    name = "iconloop",
    version,
    about = "Icon design lens→render→policy→discourse→quantify loop (ported from the Loop-Suite pattern)"
)]
struct Cli {
    #[arg(long, default_value = "claude", global = true)]
    claude_bin: String,
    #[arg(long, value_enum, default_value = "claude", global = true)]
    backend: Backend,
    #[arg(long, global = true)]
    model: Option<String>,
    #[arg(long, default_value_t = 2, global = true)]
    retries: u32,
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug)]
struct DesignArgs {
    #[arg(long)]
    spec: PathBuf,
    #[arg(long, default_value = "runs/design")]
    out: PathBuf,
    #[arg(long, default_value_t = 3)]
    concurrency: usize,
}

#[derive(Args, Debug)]
struct RefineArgs {
    #[arg(long)]
    spec: PathBuf,
    #[arg(long)]
    prior: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 3)]
    concurrency: usize,
    /// For ablation: generates a completely fresh result without showing the persona the previous
    /// round's SVG or critique text.
    /// dead_concepts (shared memory) is kept as-is — this exists to verify, by comparing against
    /// results produced from dead_concepts alone, whether "per-persona critique feedback actually
    /// contributes anything" (reflecting Kamoi et al. arXiv:2406.01297's point that reported
    /// self-refine improvements may in fact stem from other factors).
    #[arg(long)]
    no_critique: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Runs the entire pipeline from scratch with a new brief.
    Design(DesignArgs),
    /// Generates the next round by incorporating discourse critique into a previous run's (state.json) results.
    Refine(RefineArgs),
    /// Re-validates a saved state against the policy gate only, without calling the LLM.
    Validate {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        run: PathBuf,
    },
}

fn main() {
    if let Err(error) = real_main() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn build_llm(cli: &Cli) -> Result<Llm> {
    match cli.backend {
        Backend::Claude => Ok(Llm::claude_cli(
            cli.claude_bin.clone(),
            cli.model.clone(),
            cli.retries,
            cli.verbose,
        )),
        Backend::Openrouter => Llm::openrouter(cli.model.clone(), cli.retries, cli.verbose),
    }
}

/// OpenRouter llm dedicated to discourse critics — on failure (e.g. missing key), returns None
/// and just logs a warning.
/// Rather than silently shrinking the panel down to claude alone, logs why it shrank.
fn build_openrouter_critic_llm(cli: &Cli, spec: &Spec) -> Option<Llm> {
    if !spec.critic_backends.iter().any(|b| b == "openrouter") {
        return None;
    }
    match Llm::openrouter(Some(spec.openrouter_critic_model.clone()), cli.retries, cli.verbose) {
        Ok(llm) => Some(llm),
        Err(error) => {
            eprintln!(
                "Warning: OpenRouter critic backend unavailable ({error}) — the openrouter slot in \
                 critic_backends has also been replaced with claude (reduced panel diversity; quantify.rs logs a warning in the report)"
            );
            None
        }
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Design(args) => {
            let lens_llm = build_llm(&cli)?;
            let spec = Spec::load(&args.spec)?;
            let claude_llm = Llm::claude_cli(cli.claude_bin.clone(), None, cli.retries, cli.verbose);
            let openrouter_llm = build_openrouter_critic_llm(&cli, &spec);
            prepare_out(&args.out)?;
            run_pipeline(
                &lens_llm,
                &claude_llm,
                openrouter_llm.as_ref(),
                &spec,
                1,
                String::new(),
                &args.out,
                args.concurrency,
            )
        }
        Command::Refine(args) => {
            let lens_llm = build_llm(&cli)?;
            let spec = Spec::load(&args.spec)?;
            let claude_llm = Llm::claude_cli(cli.claude_bin.clone(), None, cli.retries, cli.verbose);
            let openrouter_llm = build_openrouter_critic_llm(&cli, &spec);
            let prior = state::load(&args.prior)?;
            prepare_out(&args.out)?;
            run_pipeline_refine(
                &lens_llm,
                &claude_llm,
                openrouter_llm.as_ref(),
                &spec,
                &prior,
                &args.out,
                args.concurrency,
                args.no_critique,
            )
        }
        Command::Validate { spec, run } => {
            let spec = Spec::load(spec)?;
            let state = state::load(run)?;
            let render_dir = run.join("render");
            for candidate in &state.candidates {
                let renders = render::render_all(&candidate.svg, &spec.render_sizes, &render_dir, &candidate.id)?;
                let report = policy::evaluate(&spec, &candidate.id, &candidate.svg, &renders)?;
                println!(
                    "{}: {}",
                    candidate.id,
                    if report.overall_pass { "PASS" } else { "FAIL" }
                );
                for check in &report.checks {
                    println!("  [{}] {}: {}", check.status.label(), check.id, check.evidence);
                }
            }
            Ok(())
        }
    }
}

fn prepare_out(path: &Path) -> Result<()> {
    if path.join("state.json").exists() {
        anyhow::bail!(
            "Not overwriting existing run results: {} (use a new --out or refine instead)",
            path.display()
        );
    }
    std::fs::create_dir_all(path).with_context(|| format!("Failed to create output directory: {}", path.display()))
}

#[allow(clippy::too_many_arguments)]
fn run_pipeline(
    lens_llm: &Llm,
    claude_llm: &Llm,
    openrouter_llm: Option<&Llm>,
    spec: &Spec,
    round: usize,
    dead_concepts: String,
    out: &Path,
    concurrency: usize,
) -> Result<()> {
    run_pipeline_inner(
        lens_llm,
        claude_llm,
        openrouter_llm,
        spec,
        round,
        dead_concepts,
        &[],
        out,
        concurrency,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_pipeline_refine(
    lens_llm: &Llm,
    claude_llm: &Llm,
    openrouter_llm: Option<&Llm>,
    spec: &Spec,
    prior: &state::State,
    out: &Path,
    concurrency: usize,
    no_critique: bool,
) -> Result<()> {
    // Spawns a fresh lens each round (no persistent agent identity), but injects the previous
    // round's raw SVG + critique text directly into the prompt — the Reflexion/GEPA pattern
    // (verified via research, see MEMORY).
    // With --no-critique, this per-persona feedback is left empty for ablation (dead_concepts shared memory is kept).
    let prior_info: Vec<(String, String, String)> = if no_critique {
        println!("  [ablation] --no-critique — omitting per-persona previous SVG/critique feedback, keeping only dead_concepts");
        Vec::new()
    } else {
        prior
            .candidates
            .iter()
            .map(|candidate| {
                let flaws: Vec<String> = prior
                    .discourse
                    .iter()
                    .filter_map(|round| {
                        round
                            .items
                            .iter()
                            .find(|item| item.candidate_id == candidate.id)
                            .map(|item| format!("{} (first impression: {})", item.biggest_flaw, item.blind_read))
                    })
                    .collect();
                (candidate.id.clone(), candidate.svg.clone(), flaws.join(" / "))
            })
            .collect()
    };

    run_pipeline_inner(
        lens_llm,
        claude_llm,
        openrouter_llm,
        spec,
        prior.round + 1,
        prior.dead_concepts.clone(),
        &prior_info,
        out,
        concurrency,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_pipeline_inner(
    lens_llm: &Llm,
    claude_llm: &Llm,
    openrouter_llm: Option<&Llm>,
    spec: &Spec,
    round: usize,
    dead_concepts: String,
    prior_info: &[(String, String, String)],
    out: &Path,
    concurrency: usize,
) -> Result<()> {
    println!("icon-loop starting (round {round}) — {}", spec.name);

    // 1) lens — fully independent generation per persona (par_map)
    let items: Vec<(Persona, Option<(String, String)>)> = spec
        .personas
        .iter()
        .map(|persona| {
            let prior = prior_info
                .iter()
                .find(|(id, _, _)| id == &persona.id)
                .map(|(_, svg, crit)| (svg.clone(), crit.clone()));
            (persona.clone(), prior)
        })
        .collect();

    let dead_concepts_ref = dead_concepts.as_str();
    let candidates = par_map(concurrency, items, |(persona, prior)| {
        let prior_ref = prior.as_ref().map(|(svg, crit)| (svg.as_str(), crit.as_str()));
        let candidate = lens::run(lens_llm, spec, &persona, dead_concepts_ref, prior_ref)?;
        println!("  lens done: {} ({})", persona.persona_name, persona.id);
        Ok(candidate)
    })?;
    anyhow::ensure!(!candidates.is_empty(), "No candidates were generated");

    // 2) render + policy — deterministic, no LLM involved
    let render_dir = out.join("render");
    let mut policies = Vec::new();
    for candidate in &candidates {
        let renders = render::render_all(&candidate.svg, &spec.render_sizes, &render_dir, &candidate.id)?;
        let report = policy::evaluate(spec, &candidate.id, &candidate.svg, &renders)?;
        println!(
            "  policy {}: {}",
            candidate.id,
            if report.overall_pass { "PASS" } else { "FAIL" }
        );
        policies.push(report);
    }

    // 3) anonymization — copy to candidate_N filenames without exposing persona names
    let blind_dir = out.join("blind");
    std::fs::create_dir_all(&blind_dir)?;
    let mut blind_map = Vec::new();
    for (i, candidate) in candidates.iter().enumerate() {
        let blind_id = format!("candidate_{}", i + 1);
        for &size in &spec.render_sizes {
            let src = render_dir.join(format!("{}_{}.png", candidate.id, size));
            let dst = blind_dir.join(format!("{blind_id}_{size}.png"));
            std::fs::copy(&src, &dst)
                .with_context(|| format!("Blind copy failed: {} -> {}", src.display(), dst.display()))?;
        }
        blind_map.push((blind_id, candidate.id.clone()));
    }
    let blind_ids: Vec<String> = blind_map.iter().map(|(b, _)| b.clone()).collect();

    // 4) discourse — N independent blind critics (par_map, they never see each other's output).
    // Each critic is assigned a different provider via spec.critic_backends — if they were all
    // the same model, this wouldn't be an "independent panel" but just repeated calls to the
    // same model (see quantify.rs's diversity_note).
    let critic_indices: Vec<usize> = (0..spec.n_critics).collect();
    let raw_rounds = par_map(concurrency, critic_indices, |i| {
        let critic_llm = pick_critic_llm(spec, i, claude_llm, openrouter_llm);
        let round = discourse::run_one(critic_llm, spec, &blind_ids, &blind_dir, i)?;
        println!(
            "  discourse critic {} done (provider={})",
            i + 1,
            critic_llm.provider_label
        );
        Ok(round)
    })?;

    let unblind = |blind_id: &str| -> String {
        blind_map
            .iter()
            .find(|(b, _)| b == blind_id)
            .map(|(_, real)| real.clone())
            .unwrap_or_else(|| blind_id.to_string())
    };
    let discourse_rounds: Vec<models::CritiqueRound> = raw_rounds
        .into_iter()
        .map(|mut round| {
            for item in &mut round.items {
                item.candidate_id = unblind(&item.candidate_id);
            }
            round.ranking = round.ranking.iter().map(|id| unblind(id)).collect();
            round
        })
        .collect();

    // 5) quantify — Borda count, deterministic
    let candidate_ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let quant = quantify::borda_count(&candidate_ids, &discourse_rounds);
    println!(
        "\nDone — winner: {} (Borda {}pt)",
        quant.winner,
        quant.borda.first().map(|(_, s)| *s).unwrap_or(0)
    );
    println!("  {}", quant.critic_diversity_note);
    if let Some(warning) = &quant.unanimous_warning {
        println!("  ⚠️ {warning}");
    }
    for note in &quant.minority_opinions {
        println!("  minority opinion: {note}");
    }

    let new_dead_concepts = accumulate_dead_concepts(&dead_concepts, round, &candidates, &discourse_rounds, &quant);

    let state = state::State {
        round,
        spec_name: spec.name.clone(),
        dead_concepts: new_dead_concepts,
        candidates: candidates.clone(),
        policies: policies.clone(),
        discourse: discourse_rounds.clone(),
        quant: quant.clone(),
    };
    state::write(out, &state)?;
    let report_path = report::write(out, round, &candidates, &policies, &discourse_rounds, &quant)?;
    println!("Report: {}", report_path.display());
    println!("Next round: iconloop refine --spec <spec.toml> --prior {} --out <new-out>", out.display());
    Ok(())
}

/// Accumulates the raw critique text of the Borda-lowest-ranked candidate as-is into the next
/// round's prompt — passing along the text itself, not just the score (the GEPA "actionable side
/// information" pattern, verified via research).
///
/// Attaches how many out of how many critics agreed with each point (e.g. "flagged by 2/3") —
/// reflecting SycEval's (arXiv:2502.08177) warning about regressive correction: so the next
/// round's persona doesn't over-correct something that's actually fine based on a single critic's
/// false positive, we pass along how many critics agreed with each point as grounds for judgment.
fn accumulate_dead_concepts(
    prior: &str,
    round: usize,
    candidates: &[models::IconCandidate],
    discourse: &[models::CritiqueRound],
    quant: &models::QuantResult,
) -> String {
    let mut out = String::from(prior);
    if let Some((last_id, score)) = quant.borda.last() {
        if let Some(candidate) = candidates.iter().find(|c| &c.id == last_id) {
            let n_critics = discourse.len();
            out.push_str(&format!(
                "\n\n### round {round} lowest-ranked: {}({}) — Borda {score}pt (tallied from {n_critics} critics)\n",
                candidate.persona_name, candidate.id
            ));
            let mut flaw_lines: Vec<String> = Vec::new();
            for round_result in discourse {
                if let Some(item) = round_result.items.iter().find(|i| &i.candidate_id == last_id) {
                    if item.biggest_flaw.trim() != "none" && !item.biggest_flaw.trim().is_empty() {
                        flaw_lines.push(format!(
                            "- [{}] {} (first impression: {})",
                            round_result.provider, item.biggest_flaw, item.blind_read
                        ));
                    }
                }
            }
            let corroborated = flaw_lines.len();
            out.push_str(&format!(
                "{corroborated} flagged issue(s) (gauge how many critics agreed from each line's provider label — \
                 a single provider flagging something may be a false positive, so don't over-correct; prioritize issues where multiple providers overlap):\n"
            ));
            for line in flaw_lines {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

/// The llm corresponding to critic_backends[index % len] for the given critic index. If
/// openrouter is assigned but unavailable (missing key), this isn't a silent fallback to claude —
/// build_openrouter_critic_llm has already logged a warning, and quantify::diversity_note logs it
/// again in the final report.
fn pick_critic_llm<'a>(
    spec: &Spec,
    critic_index: usize,
    claude_llm: &'a Llm,
    openrouter_llm: Option<&'a Llm>,
) -> &'a Llm {
    let backends = &spec.critic_backends;
    if backends.is_empty() {
        return claude_llm;
    }
    match backends[critic_index % backends.len()].as_str() {
        "openrouter" => openrouter_llm.unwrap_or(claude_llm),
        _ => claude_llm,
    }
}

fn par_map<T, R, F>(concurrency: usize, items: Vec<T>, function: F) -> Result<Vec<R>>
where
    T: Send,
    R: Send,
    F: Fn(T) -> Result<R> + Sync,
{
    let concurrency = concurrency.max(1);
    let mut output = Vec::new();
    let mut rest = items;
    while !rest.is_empty() {
        let take = concurrency.min(rest.len());
        let chunk = rest.drain(..take).collect::<Vec<_>>();
        let results = std::thread::scope(|scope| {
            let handles = chunk
                .into_iter()
                .map(|item| scope.spawn(|| function(item)))
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().map_err(|_| anyhow!("worker thread panicked")).and_then(|inner| inner))
                .collect::<Vec<Result<R>>>()
        });
        for result in results {
            output.push(result?);
        }
    }
    Ok(output)
}
