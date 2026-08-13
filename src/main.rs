use astrobib::library::{find_manuscript_db, MergedLibrary};
use astrobib::query::{self, QueryContext};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "astrobib", version, about = "ADS-native literature manager for astrophysics")]
#[command(args_conflicts_with_subcommands = false)]
struct Cli {
    /// Tier-2 local bib root to operate on (a directory holding bib/,
    /// created lazily on first write). Defaults to walk-up from cwd.
    #[arg(value_name = "LIBRARY_DIR")]
    path: Option<std::path::PathBuf>,
    /// Use a different GLOBAL (tier-1) library root (wins over
    /// $ASTROBIB_LIBRARY). Caches and state.json are unaffected.
    #[arg(long, global = true, value_name = "PATH")]
    library: Option<std::path::PathBuf>,
    /// Start with the global tier hidden (local-only reads and writes).
    #[arg(long, global = true)]
    no_global: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List library entries, newest first
    List {
        /// How many entries to print
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Search the local library, or ADS with --ads
    Search {
        /// The filter language (author:, ^first, abs:"…", year:2015-2020,
        /// is:pdf, tag:, OR, leading -); passed to ADS unchanged with --ads
        query: String,
        /// How many results to print
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
        /// Search ADS instead of the local library
        #[arg(long)]
        ads: bool,
    },
    /// Add a paper by ADS bibcode (or pasted ADS URL)
    Add {
        /// An ADS bibcode, or any ADS URL naming one
        bibcode: String,
        /// Overwrite the entry if this paper is already in the library
        #[arg(short, long)]
        force: bool,
        /// Also write the paper into the global (tier-1) library. The
        /// default is the local (tier-2) one whenever there is one.
        #[arg(long)]
        global: bool,
    },
    /// Print the BibTeX entry for a cite key (full or shortened)
    Show {
        /// A cite key, an unambiguous prefix of one, or a bibcode
        key: String,
    },
    /// Show the resolved environment, or set a value (token, email)
    Config {
        /// Field to set: ads_token | email
        #[arg(value_parser = ["ads_token", "email"], requires = "value")]
        key: Option<String>,
        /// New value for the field
        value: Option<String>,
    },
    /// Report what the machine-local caches cost, and where they are
    Gc,
    /// Rewrite every manuscript cite key to one uniform format and
    /// regenerate refs.bib to match
    Convert {
        /// Target key format
        #[arg(value_parser = ["bibcode", "full", "short"])]
        format: String,
        /// Report what would change without touching anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove an entry by cite key (unambiguous prefix or bibcode)
    Rm {
        /// A cite key, an unambiguous prefix of one, or a bibcode
        key: String,
        /// Remove only the local (tier-2) copy; a sole copy is rescued
        /// into the global library first
        #[arg(long)]
        local_only: bool,
    },
    /// Refresh entries whose arXiv preprint has since been published
    Update {
        /// Re-fetch every entry with an ADS record, not just preprints
        #[arg(long)]
        all: bool,
    },
    /// Import papers from a .bib file, resolving each against ADS
    Import {
        /// The .bib file to read; it is never modified
        file: std::path::PathBuf,
        /// Write to the global (tier-1) library as well as the local one
        #[arg(long, conflicts_with_all = ["global_only", "local_only"])]
        global: bool,
        /// Write only to the global (tier-1) library
        #[arg(long)]
        global_only: bool,
        /// Write only to the local (tier-2) library — the default
        /// whenever a local library exists, and an error when none does
        #[arg(long, conflicts_with = "global_only")]
        local_only: bool,
        /// Import only the entries this manuscript cites, skipping the
        /// rest untouched (needs a manuscript with .tex/.md sources)
        #[arg(long)]
        cited_only: bool,
        /// Rewrite the imported entries' old cite keys in the
        /// manuscript's .tex/.md sources (needs a manuscript with
        /// sources)
        #[arg(long)]
        rename_citekeys: bool,
        /// Resolve everything and report what would be written — which
        /// .bib files, to which tier, and which cites would be
        /// rewritten — without touching anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Canonicalize hand-dropped .bib files in the manuscript's bib/
    /// (re-key, rename, dedupe), then regenerate refs.bib
    #[command(visible_alias = "regularize")]
    Tidy {
        /// Report what would change without touching anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Sync the manuscript db with cited keys, then write refs.bib
    /// (TeX) and/or the rendered bibliography (markdown)
    Refs {
        /// Markdown file to update (default: main.md, or the sole .md)
        file: Option<std::path::PathBuf>,
        /// Print what would be written without touching anything
        #[arg(long)]
        dry_run: bool,
        /// Remove uncited entries from the manuscript database
        #[arg(long, conflicts_with_all = ["no_sync", "check"])]
        prune: bool,
        /// Produce refs.bib without copying cited entries into bib/ —
        /// the build-safe mode: reads the manuscript, writes only the
        /// bibliography, and never modifies its own inputs
        #[arg(long, conflicts_with = "check")]
        no_sync: bool,
        /// Report whether refs.bib is current and exit; writes nothing
        /// (exit 0 = current, 1 = stale or missing)
        #[arg(long)]
        check: bool,
        /// refs.bib output path (default: refs.bib in the manuscript root)
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
}

/// Rust starts every program with SIGPIPE ignored, which turns the
/// ordinary `astrobib list | head` into a panic ("failed printing to
/// stdout: Broken pipe") once the reader closes. Restoring the default
/// disposition makes the process die quietly on signal 13, the way
/// every other command-line tool does.
#[cfg(unix)]
fn restore_default_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}

fn main() -> anyhow::Result<()> {
    restore_default_sigpipe();
    let cli = Cli::parse();
    if let Some(p) = &cli.library {
        // one resolution path: the flag wins by shadowing the env var
        // (read before any thread starts)
        std::env::set_var("ASTROBIB_LIBRARY", p);
    }
    let local_root = match &cli.path {
        Some(p) => {
            // an explicit tier-2 root must at least be a directory
            // (bib/ inside it is created lazily on first write) — a
            // typo'd command lands here otherwise
            if !p.is_dir() {
                eprintln!(
                    "'{}' is neither an astrobib command nor a directory.\n\
                     Run astrobib --help for the command list.",
                    p.display()
                );
                std::process::exit(2);
            }
            let p = p.canonicalize().unwrap_or_else(|_| p.clone());
            Some(p)
        }
        None => find_manuscript_db(),
    };
    let t_load = std::time::Instant::now();
    let mut lib = MergedLibrary::load(local_root.as_deref())?;
    astrobib::tui::debug_startup(&format!(
        "library load {:?} ms={}",
        t_load.elapsed(),
        local_root.is_some()
    ));
    if cli.no_global && lib.manuscript.is_some() {
        lib.global_on = false;
    }
    match cli.command {
        None => astrobib::tui::run(lib),
        Some(Command::List { limit }) => {
            print_entries(&lib, |_| true, limit);
            Ok(())
        }
        Some(Command::Search { query, limit, ads }) => {
            if ads {
                // a pasted DOI or DOI URL becomes a fielded query
                let q = match astrobib::ads::doi_from_text(&query) {
                    Some(doi) => format!("doi:\"{doi}\""),
                    None => query,
                };
                // same columns as local search, with the bibcode standing
                // in for the cite key (results aren't imported yet)
                let results = astrobib::ads::search(&q, limit)?;
                for a in &results {
                    let author = a.author.join(" and ");
                    println!(
                        "{:<24} {:<6} {:<18} {}",
                        a.bibcode,
                        a.year,
                        astrobib::text::fit_authors(&author, 18),
                        truncate(&a.title, 60)
                    );
                }
                println!("{} result(s)", results.len());
                return Ok(());
            }
            let groups = query::tokenize(&query);
            let ctx = query_context(&lib);
            print_entries(&lib, |e| query::matches(&groups, e, &ctx), limit);
            Ok(())
        }
        Some(Command::Add { bibcode, force, global }) => {
            let bc = astrobib::ads::bibcode_from_url(&bibcode).unwrap_or(bibcode);
            let Some(data) = astrobib::ads::fetch_bibtex(&bc)? else {
                eprintln!("Could not fetch BibTeX for {bc}");
                std::process::exit(1);
            };
            let key = astrobib::keys::generate_key(&data);
            let (to_global, to_local) = write_tiers(&lib, global);
            // "already there" is a question about the tiers this run
            // would write, not about the library as a whole: with a
            // local db open, `add --global` on a paper you already hold
            // locally is exactly the gesture that shares it
            let held = (!to_global || lib.in_personal(&key))
                && (!to_local || lib.in_manuscript(&key));
            if held && !force {
                eprintln!("{key} already in the {}. Use --force to overwrite.", tier_label(to_global, to_local));
                std::process::exit(1);
            }
            let key = lib.save_entry_to(&data, global)?;
            let e = lib.get(&key).unwrap();
            let display = if e.short_key.is_empty() { &key } else { &e.short_key };
            println!("Added {display}  ({key}) → {}", tier_label(to_global, to_local));
            Ok(())
        }
        Some(Command::Import {
            file,
            global,
            global_only,
            local_only,
            cited_only,
            rename_citekeys,
            dry_run,
        }) => import_bib(
            &mut lib,
            &file,
            local_root.as_deref(),
            ImportOpts { global, global_only, local_only, cited_only, rename_citekeys, dry_run },
        ),
        Some(Command::Tidy { dry_run }) => {
            match local_root.clone() {
                Some(root) => tidy_bib_dir(&mut lib, &root, dry_run),
                // no bib/ anywhere: a legacy manuscript (sources plus
                // loose .bib files) can be adopted wholesale
                None => adopt_manuscript(&mut lib, dry_run),
            }
        }
        Some(Command::Refs { file, dry_run, prune, no_sync, check, output }) => {
            let Some(root) = local_root.clone() else {
                eprintln!("No local bib root here — refs needs a manuscript directory.");
                std::process::exit(1);
            };
            let mode = if check {
                RefsMode::Check
            } else if no_sync {
                RefsMode::Generate
            } else {
                RefsMode::Sync
            };
            run_refs(&mut lib, &root, file.as_deref(), output.as_deref(), prune, dry_run, mode)
        }
        Some(Command::Update { all }) => run_update(&mut lib, all),
        Some(Command::Convert { format, dry_run }) => {
            let Some(root) = local_root.clone() else {
                eprintln!("No local bib root here — convert needs a manuscript directory.");
                std::process::exit(1);
            };
            run_convert(&mut lib, &root, &format, dry_run)
        }
        Some(Command::Gc) => {
            run_gc();
            Ok(())
        }
        Some(Command::Config { key: Some(key), value: Some(value) }) => {
            astrobib::ads::save_state_field(&key, &value)?;
            println!("{key} saved.");
            Ok(())
        }
        Some(Command::Config { .. }) => {
            run_config(&lib, local_root.as_deref(), cli.library.is_some());
            Ok(())
        }
        Some(Command::Rm { key, local_only }) => {
            let Some(e) = lib.resolve(&key) else {
                let matches = lib.possible_matches(&key);
                if matches.len() > 1 {
                    eprintln!("Ambiguous key '{key}' — did you mean:");
                    for m in matches.iter().take(6) {
                        eprintln!("  {}  {}", m.short_key, truncate(m.title(), 55));
                    }
                } else {
                    eprintln!("{key} not found.");
                }
                std::process::exit(1);
            };
            let full = e.key().to_string();
            let title = truncate(e.title().trim_matches(['{', '}']), 55).to_string();
            if local_only {
                if lib.manuscript.is_none() {
                    eprintln!("--local-only needs a local (tier-2) library.");
                    std::process::exit(1);
                }
                let rescued = !lib.in_personal(&full);
                match lib.remove_from_manuscript(&full)? {
                    true => println!(
                        "Removed {full} from the local library{}  — {title}",
                        if rescued { "  (sole copy rescued into the global library)" } else { "" }
                    ),
                    false => {
                        eprintln!("{full} is not in the local library.");
                        std::process::exit(1);
                    }
                }
            } else {
                lib.remove_entry(&full)?;
                println!("Removed {full}  — {title}");
            }
            Ok(())
        }
        Some(Command::Show { key }) => match lib.resolve(&key) {
            Some(e) => {
                print!("{}", std::fs::read_to_string(&e.path)?);
                Ok(())
            }
            None => {
                let matches = lib.possible_matches(&key);
                if matches.len() > 1 {
                    eprintln!("Ambiguous key '{key}' — did you mean:");
                    for m in matches.iter().take(6) {
                        eprintln!("  {}  {}", m.short_key, truncate(m.title(), 55));
                    }
                } else {
                    eprintln!("{key} not found.");
                }
                std::process::exit(1);
            }
        },
    }
}

/// The context the local filter needs for the terms that live outside
/// the BibTeX data: is:ms / is:pdf and the pri:/cit: comparisons. The
/// default (empty) context silently matches nothing for those terms, so
/// the CLI must build a real one or `astrobib search pri:>0.5` reports
/// no results however many papers qualify.
fn query_context(lib: &MergedLibrary) -> QueryContext {
    use std::collections::{HashMap, HashSet};
    let in_ms: HashSet<String> = lib
        .manuscript
        .as_ref()
        .map(|m| m.entries().iter().map(|e| e.key().to_string()).collect())
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let metrics = astrobib::metrics::Metrics::load();
    let pri: HashMap<String, f64> = metrics
        .papers
        .iter()
        .filter_map(|(k, m)| m.effective_priority(now).map(|v| (k.clone(), v)))
        .collect();
    let cit: HashMap<String, f64> = metrics
        .papers
        .iter()
        .filter_map(|(k, m)| m.citations.map(|v| (k.clone(), v as f64)))
        .collect();
    let mut tagged: HashMap<String, Vec<String>> = HashMap::new();
    for (name, keys) in lib.tags() {
        for k in keys {
            tagged.entry(k).or_default().push(name.to_lowercase());
        }
    }
    QueryContext {
        in_manuscript: Some(Box::new(move |k: &str| in_ms.contains(k))),
        has_pdf: Some(Box::new(astrobib::library::has_cached_pdf)),
        priority: Some(Box::new(move |k: &str| pri.get(k).copied())),
        citations: Some(Box::new(move |k: &str| cit.get(k).copied())),
        tagged: Some(Box::new(move |k: &str, v: &str| {
            tagged.get(k).is_some_and(|ts| ts.iter().any(|t| t.contains(v)))
        })),
    }
}

/// astrobib update — publication-status refresh. Checks each
/// preprint-form entry (bibcode like 2024arXiv…) against ADS; when the
/// record has gained publication metadata, rewrites the entry in place
/// with canonical BibTeX. Cite keys never change; each copy's
/// user-curated keywords are preserved; every tier holding the entry is
/// refreshed. `--all` re-fetches every entry with an ADS record.
fn run_update(lib: &mut MergedLibrary, all: bool) -> anyhow::Result<()> {
    use astrobib::ads;
    struct Candidate {
        key: String,
        display: String,
        bibcode: String,
        eprint: String,
    }
    let candidates: Vec<Candidate> = lib
        .entries()
        .into_iter()
        .filter_map(|e| {
            let bibcode = ads::update_candidate(e.adsurl(), e.eprint(), all)?;
            Some(Candidate {
                key: e.key().to_string(),
                display: if e.short_key.is_empty() { e.key().to_string() } else { e.short_key.clone() },
                bibcode,
                eprint: e.eprint().to_string(),
            })
        })
        .collect();
    if candidates.is_empty() {
        println!("No preprint-form entries to check.");
        return Ok(());
    }
    println!(
        "Checking {} entr{} against ADS…",
        candidates.len(),
        if candidates.len() == 1 { "y" } else { "ies" }
    );
    let mut updated: Vec<String> = vec![];
    let (mut still, mut failed) = (0usize, 0usize);
    for c in &candidates {
        let results = match ads::search(&ads::refresh_query(&c.eprint, &c.bibcode), 1) {
            Ok(r) => r,
            Err(e) => {
                failed += 1;
                println!("  {}: {e}", c.key);
                continue;
            }
        };
        let Some(hit) = results.first() else {
            still += 1;
            continue;
        };
        if hit.bibcode == c.bibcode && !all {
            still += 1;
            continue;
        }
        let data = match ads::fetch_bibtex(&hit.bibcode) {
            Ok(Some(d)) => d,
            Ok(None) => {
                failed += 1;
                continue;
            }
            Err(e) => {
                failed += 1;
                println!("  {}: {e}", c.key);
                continue;
            }
        };
        lib.update_entry(&c.key, &data)?;
        if hit.bibcode != c.bibcode {
            let venue = ads::published_where(&data);
            updated.push(format!("{}  →  {venue}", c.display).trim_end().to_string());
        } else {
            updated.push(c.display.clone());
        }
    }
    for line in &updated {
        println!("  {line}");
    }
    let mut summary = format!(
        "\n{} checked · {} updated · {still} still preprint",
        candidates.len(),
        updated.len()
    );
    if failed > 0 {
        summary.push_str(&format!(" · {failed} failed"));
    }
    if let Some(use_) = ads::quota_summary() {
        summary.push_str(&format!("  · ADS use today {use_}"));
    }
    println!("{summary}");
    Ok(())
}

/// Byte counts humans can read: KB below a megabyte, else MB.
fn human_bytes(n: u64) -> String {
    if n < 1_000_000 {
        format!("{:.0} KB", n as f64 / 1e3)
    } else {
        format!("{:.1} MB", n as f64 / 1e6)
    }
}

/// File count and total size of a directory's immediate contents.
fn dir_usage(dir: &std::path::Path) -> (usize, u64) {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .fold((0usize, 0u64), |(n, b), m| (n + 1, b + m.len()))
        })
        .unwrap_or((0, 0))
}

fn file_size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// astrobib gc — a report, never a reaper: what the machine-local
/// caches cost, where they are, and which of them you may delete.
///
/// It deliberately does not hunt for orphans. A cite key denotes the
/// same paper in every library on the machine, libraries are found by
/// walking up from wherever you happen to be, and the caches are shared
/// across all of them — so "does any library still hold this key?" is
/// not a question this process can answer. A library on an unmounted
/// volume, or one simply not visited this month, would make live papers
/// look like garbage. Reclaiming the caches wholesale loses nothing
/// (every byte is one ADS query or one download away from returning),
/// so that is the operation astrobib recommends, and it belongs to the
/// user's own rm rather than to a subcommand.
fn run_gc() {
    let cache = astrobib::library::cache_dir();
    let pdfs = astrobib::library::pdf_cache_dir();
    let (n_pdf, pdf_bytes) = dir_usage(&pdfs);
    let queries = astrobib::tabs::cache_file();
    let metrics_path = astrobib::metrics::metrics_file();
    let n_papers = astrobib::metrics::Metrics::load().papers.len();

    println!("Machine-local derived data — none of it is your bibliography.\n");
    println!(
        "PDF cache        {}  ({n_pdf} file(s), {})",
        pdfs.display(),
        human_bytes(pdf_bytes)
    );
    println!(
        "query cache      {}  ({})",
        queries.display(),
        human_bytes(file_size(&queries))
    );
    println!(
        "metrics          {}  ({n_papers} paper(s), {})",
        metrics_path.display(),
        human_bytes(file_size(&metrics_path))
    );
    println!(
        "\nThe cache directory is disposable: rm -rf {} reclaims all of it,\n\
         and everything in it comes back on demand — one ADS query, one\n\
         download. astrobib never deletes it for you.",
        cache.display()
    );
    println!(
        "\nmetrics.json is not cache: priorities are hand-curated, they halve\n\
         every week untouched (so stale ones fade on their own), and the file\n\
         costs kilobytes. It is never removed automatically."
    );
}

/// astrobib config — the resolved environment, for humans debugging it.
fn run_config(lib: &MergedLibrary, local_root: Option<&std::path::Path>, via_flag: bool) {
    let lib_src = if via_flag {
        "--library"
    } else if std::env::var("ASTROBIB_LIBRARY").is_ok() {
        "$ASTROBIB_LIBRARY"
    } else {
        "default"
    };
    println!("astrobib {}", env!("CARGO_PKG_VERSION"));
    println!(
        "global library   {}  ({lib_src}, {} entries)",
        lib.personal.root.display(),
        lib.personal.entries().len()
    );
    match (local_root, &lib.manuscript) {
        (Some(root), Some(ms)) => {
            println!(
                "local library    {}  ({} entries)",
                root.display(),
                ms.entries().len()
            );
        }
        _ => println!("local library    none (no bib/ found in cwd ancestors)"),
    }
    let token_src = if std::env::var("ADS_API_TOKEN").is_ok_and(|t| !t.is_empty()) {
        "from $ADS_API_TOKEN"
    } else if astrobib::ads::get_token().is_some() {
        "from state.json"
    } else {
        "MISSING — press S in the TUI to set one"
    };
    println!("ADS token        {token_src}");
    println!(
        "email            {}",
        astrobib::ads::get_email().unwrap_or_else(|| "not set".to_string())
    );
    // ADS meters each endpoint on its own daily allowance, so the lines
    // are per endpoint. The figures are whatever the last call to each
    // one reported — no request is made to produce this report.
    let quotas = astrobib::ads::quotas();
    if quotas.is_empty() {
        println!("ADS API use      no calls recorded yet");
    } else {
        for (i, (endpoint, q)) in quotas.iter().enumerate() {
            let label = if i == 0 { "ADS API use" } else { "" };
            println!("{label:<16} {endpoint:<9}{}", q.describe());
        }
    }
    let cache = astrobib::library::pdf_cache_dir();
    let (n, bytes) = dir_usage(&cache);
    println!(
        "PDF cache        {}  ({n} files, {})",
        cache.display(),
        human_bytes(bytes)
    );
    println!(
        "metrics          {} paper(s)  (see astrobib gc for cache sizes)",
        astrobib::metrics::Metrics::load().papers.len()
    );
}

/// Rewrite old→new cite keys in every manuscript source, reporting per
/// file. TeX cites live in `\cite…{}` braces and markdown ones are
/// pandoc `@Key` or Obsidian `[[Key]]`; a cite means the same thing in
/// both, so a rewrite that covered only one would leave half a
/// manuscript pointing at keys that no longer exist.
///
/// Returns the number of cites changed — or, under `dry_run`, the number
/// that would be, found by the same code that would apply them.
fn rewrite_citations(
    root: &std::path::Path,
    renames: &[(String, String)],
    dry_run: bool,
) -> usize {
    use std::collections::HashMap;
    let map: HashMap<&str, &str> =
        renames.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let lookup = |k: &str| map.get(k).map(|s| s.to_string());
    let mut total = 0usize;
    for f in astrobib::export::manuscript_source_files(root) {
        let is_md = f.extension().is_some_and(|x| x == "md");
        let res = if is_md {
            astrobib::export::convert_md_citations(&f, lookup, dry_run)
        } else {
            astrobib::export::convert_citations(&f, lookup, dry_run)
        };
        match res {
            Ok(0) => {}
            Ok(n) => {
                total += n;
                let verb = if dry_run { "would rewrite" } else { "rewrote" };
                println!("  {verb} {n} cite(s) in {}", f.display());
            }
            Err(e) => eprintln!("  could not rewrite {}: {e}", f.display()),
        }
    }
    total
}

/// astrobib convert — every cited key to one uniform format (bibcode,
/// full key, or shortest unambiguous key), rewritten in the sources,
/// then refs.bib regenerated to match.
fn run_convert(
    lib: &mut MergedLibrary,
    root: &std::path::Path,
    format: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    let files = astrobib::export::manuscript_tex_files(root);
    if files.is_empty() {
        eprintln!("No .tex sources found in {}.", root.display());
        std::process::exit(1);
    }
    let cited = astrobib::export::scan_tex_files(&files);
    let mut renames: Vec<(String, String)> = vec![];
    let mut unresolved: Vec<String> = vec![];
    for c in &cited {
        match lib.resolve_citation(c).1 {
            Some(e) => {
                let target = match format {
                    "bibcode" => {
                        e.bibcode().map(str::to_string).unwrap_or_else(|| e.key().to_string())
                    }
                    "short" => {
                        if e.short_key.is_empty() {
                            e.key().to_string()
                        } else {
                            e.short_key.clone()
                        }
                    }
                    _ => e.key().to_string(),
                };
                if *c != target {
                    renames.push((c.clone(), target));
                }
            }
            None => unresolved.push(c.clone()),
        }
    }
    if dry_run {
        for (old, new) in &renames {
            println!("  {old} → {new}");
        }
        println!("would rewrite {} cite key(s)", renames.len());
    } else if renames.is_empty() {
        println!("all {} cited key(s) already in {format} form", cited.len());
    } else {
        rewrite_citations(root, &renames, false);
        println!("{} cite key(s) converted to {format} form", renames.len());
        run_refs(lib, root, None, None, false, false, RefsMode::Sync)?;
    }
    for c in &unresolved {
        eprintln!("unresolved (left alone): {c}");
    }
    Ok(())
}

/// astrobib tidy with no bib/ in sight — adopt a legacy manuscript:
/// a directory holding .tex/.md sources and one or more loose .bib
/// files (its own refs.bib, arbitrary cite keys). Creates bib/,
/// resolves every entry against ADS, rewrites the old keys inside the
/// sources' \cite braces, and regenerates refs.bib canonically.
fn adopt_manuscript(lib: &mut MergedLibrary, dry_run: bool) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let mut loose: Vec<std::path::PathBuf> = std::fs::read_dir(&root)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bib"))
        .collect();
    loose.sort();
    let has_sources = !astrobib::export::manuscript_tex_files(&root).is_empty()
        || !astrobib::export::manuscript_md_files(&root).is_empty();
    if loose.is_empty() || !has_sources {
        eprintln!(
            "Nothing to tidy here: no bib/ directory, and adoption needs manuscript\n\
             sources (.tex/.md) plus at least one loose .bib file in {}.",
            root.display()
        );
        std::process::exit(1);
    }
    println!(
        "Adopting {}: {} loose .bib file(s) → bib/",
        root.display(),
        loose.len()
    );
    // the manuscript tier now points here (bib/ is created on write)
    let global_on = lib.global_on;
    *lib = MergedLibrary::load(Some(&root))?;
    lib.global_on = global_on;
    let mut renames: Vec<(String, String)> = vec![];
    let (mut adopted, mut skipped) = (0usize, 0usize);
    for path in &loose {
        let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        for mut data in astrobib::bib::parse_entries(&text) {
            let orig_key = data.get("ID").cloned().unwrap_or_else(|| "?".to_string());
            if orig_key != astrobib::keys::generate_key(&data) {
                if dry_run {
                    println!("  {fname}: would resolve {orig_key} against ADS");
                    continue;
                }
                match astrobib::ads::lookup_entry(&data) {
                    Ok(r) => data = r,
                    Err(reason) => {
                        println!("  ⚠ {fname}: {orig_key} left alone — {reason}");
                        skipped += 1;
                        continue;
                    }
                }
            } else if dry_run {
                println!("  {fname}: {orig_key} is canonical — would adopt directly");
                continue;
            }
            let key = astrobib::keys::generate_key(&data);
            let existing = data
                .get("adsurl")
                .and_then(|u| astrobib::pdf::bibcode_from_adsurl(u))
                .and_then(|b| lib.get_by_bibcode(b))
                .map(|e| e.key().to_string());
            let final_key = match existing {
                Some(k) => k, // already present (possibly under another key)
                None => {
                    let mut d = data.clone();
                    d.insert("ID".to_string(), key.clone());
                    lib.save_entry(&d)?
                }
            };
            let short = lib
                .get(&final_key)
                .map(|e| if e.short_key.is_empty() { final_key.clone() } else { e.short_key.clone() })
                .unwrap_or_else(|| final_key.clone());
            if orig_key != short && orig_key != final_key {
                println!("  {fname}: {orig_key} → {short}");
                renames.push((orig_key, short));
            } else {
                println!("  {fname}: {final_key}");
            }
            adopted += 1;
        }
    }
    if dry_run {
        println!("(dry run — nothing written)");
        return Ok(());
    }
    // stale in-memory paths after the writes: reload before refs
    let global_on = lib.global_on;
    *lib = MergedLibrary::load(Some(&root))?;
    lib.global_on = global_on;
    if !renames.is_empty() {
        rewrite_citations(&root, &renames, false);
    }
    println!(
        "\n{adopted} entr{} adopted, {skipped} left alone.",
        if adopted == 1 { "y" } else { "ies" }
    );
    // an adoption that adopted nothing is a failure, not a quiet
    // success: pressing on would regenerate refs.bib from an empty
    // manuscript db, overwriting the very .bib file we failed to read
    if adopted == 0 {
        eprintln!(
            "\nNothing adopted — {} left unchanged.\n\
             Foreign cite keys are resolved against ADS; set a token with\n\
             astrobib config ads_token <token> (or $ADS_API_TOKEN) and retry.",
            root.display()
        );
        std::process::exit(1);
    }
    let superseded: Vec<_> = loose
        .iter()
        .filter(|p| p.file_name().is_some_and(|f| f != "refs.bib"))
        .collect();
    if !superseded.is_empty() {
        println!("Loose .bib file(s) now superseded by bib/ (left in place):");
        for p in superseded {
            println!("  {}", p.display());
        }
    }
    println!();
    run_refs(lib, &root, None, None, false, false, RefsMode::Sync)
}

/// astrobib tidy — co-author interop. Colleagues without astrobib add
/// references by dropping raw ADS BibTeX into bib/ under any filename
/// and key; this canonicalizes those files (reproducible-key fast path,
/// else the ADS lookup ladder), renames them to {Key}.bib, dedupes
/// against the library, prints cite-key replacement one-liners, and
/// regenerates refs.bib.
fn tidy_bib_dir(
    lib: &mut MergedLibrary,
    root: &std::path::Path,
    dry_run: bool,
) -> anyhow::Result<()> {
    let bib_dir = root.join("bib");
    // (path, entries) for files that aren't canonical one-entry {Key}.bib
    let mut foreign: Vec<(std::path::PathBuf, Vec<astrobib::bib::Data>)> = vec![];
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&bib_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "bib"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let entries = astrobib::bib::parse_entries(&text);
        let canonical = entries.len() == 1
            && entries.first().is_some_and(|d| {
                let id = d.get("ID").map(String::as_str).unwrap_or("");
                id == astrobib::keys::generate_key(d)
                    && path.file_name().is_some_and(|f| {
                        f.to_string_lossy() == format!("{id}.bib")
                    })
            });
        if !canonical && !entries.is_empty() {
            foreign.push((path, entries));
        }
    }
    // tags/ is canonicalized on the same pass: sorted, deduped, comments
    // kept. Never filtered against the library — a key naming a paper
    // not yet imported is the format working, and tidy dropping curated
    // lines is exactly what the design forbids.
    if !dry_run {
        let moved = astrobib::tags::tidy(root);
        if !moved.is_empty() {
            println!("Tidied tags/: {}", moved.join(", "));
        }
    }
    if foreign.is_empty() {
        println!("bib/ is already canonical.");
        return run_refs(lib, root, None, None, false, dry_run, RefsMode::Sync);
    }
    let mut renames: Vec<(String, String)> = vec![];
    let (mut tidied, mut skipped) = (0usize, 0usize);
    for (path, entries) in foreign {
        let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut all_ok = true;
        let mut resolved: Vec<astrobib::bib::Data> = vec![];
        for mut data in entries {
            let orig_key = data.get("ID").cloned().unwrap_or_else(|| "?".to_string());
            if orig_key != astrobib::keys::generate_key(&data) {
                if dry_run {
                    println!("  {fname}: would resolve {orig_key} against ADS");
                    continue;
                }
                match astrobib::ads::lookup_entry(&data) {
                    Ok(r) => data = r,
                    Err(reason) => {
                        println!("  ⚠ {fname}: {orig_key} left alone — {reason}");
                        all_ok = false;
                        skipped += 1;
                        continue;
                    }
                }
            }
            resolved.push(data);
        }
        if dry_run {
            continue;
        }
        for data in &resolved {
            let key = astrobib::keys::generate_key(data);
            let orig_key = data.get("ID").cloned().unwrap_or_default();
            // an entry already canonical elsewhere: this copy is a dupe
            let dup = lib
                .manuscript
                .as_ref()
                .and_then(|ms| ms.get(&key))
                .is_some_and(|e| e.path != path);
            if !dup {
                let mut d = data.clone();
                d.insert("ID".to_string(), key.clone());
                lib.save_entry(&d)?; // both tiers, canonical {Key}.bib
            }
            let short = lib
                .get(&key)
                .map(|e| if e.short_key.is_empty() { key.clone() } else { e.short_key.clone() })
                .unwrap_or_else(|| key.clone());
            if orig_key != short && orig_key != key {
                renames.push((orig_key.clone(), short.clone()));
                println!("  {fname}: {orig_key} → {short}{}", if dup { "  (duplicate — dropped)" } else { "" });
            } else {
                println!("  {fname} → {key}.bib");
            }
            tidied += 1;
        }
        // the canonical copies exist now; the foreign file goes
        if all_ok {
            std::fs::remove_file(&path)?;
        }
    }
    if dry_run {
        println!("(dry run — nothing written)");
        return Ok(());
    }
    // in-memory state has stale paths for the removed files: reload
    let global_on = lib.global_on;
    *lib = MergedLibrary::load(Some(root))?;
    lib.global_on = global_on;
    println!("\n{tidied} entr{} canonicalized, {skipped} left alone.", if tidied == 1 { "y" } else { "ies" });
    if !renames.is_empty() {
        println!("\nRe-keyed:");
        for (old, new) in &renames {
            println!("  {old} → {new}");
        }
        rewrite_citations(root, &renames, false);
    }
    println!();
    run_refs(lib, root, None, None, false, false, RefsMode::Sync)
}

/// astrobib refs — the manuscript sync flow, for TeX and markdown
/// manuscripts alike. Cited keys missing from the manuscript db are
/// copied in from the personal library; keys found nowhere are reported;
/// refs.bib is generated from the manuscript db (each entry keyed by the
/// string actually cited); a markdown manuscript also gets its rendered
/// bibliography.
/// The Check counterpart for a markdown manuscript: is the rendered
/// bibliography block between the markers what we would write?
fn check_markdown(
    lib: &MergedLibrary,
    _root: &std::path::Path,
    file: Option<&std::path::Path>,
    md_files: &[std::path::PathBuf],
    cited: &[String],
) -> anyhow::Result<()> {
    use astrobib::export;
    let Some(target) = (match file {
        Some(f) => Some(f.to_path_buf()),
        None => md_files
            .first()
            .filter(|f| md_files.len() == 1 || f.ends_with("main.md"))
            .cloned(),
    }) else {
        return Ok(()); // no markdown manuscript to check
    };
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let entries: Vec<_> = cited
        .iter()
        .filter_map(|c| lib.resolve_citation(c).1)
        .filter(|e| seen.insert(e.key().to_string()))
        .collect();
    let block = export::render_md_bibliography(&entries);
    let text = std::fs::read_to_string(&target).unwrap_or_default();
    if text.contains(&block) {
        println!("{} bibliography is current", target.display());
        Ok(())
    } else {
        eprintln!(
            "{} bibliography is out of date — run: astrobib refs",
            target.display()
        );
        std::process::exit(1);
    }
}

/// Set a file's mtime to now without altering a byte of it.
fn stamp(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)?
        .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
}

/// What a `refs` invocation is being asked to do. The three modes exist
/// because three different callers want three different contracts:
///
/// * `Sync` — a person at a terminal: pull cited entries into `bib/`,
///   write the bibliography, report what changed. Mutates.
/// * `Generate` — a build system: produce `refs.bib` from what is
///   already present and touch nothing else, so a recipe never writes
///   into its own prerequisites. Always stamps the file's mtime, since
///   make judges freshness by timestamp and would otherwise re-run the
///   rule forever.
/// * `Check` — a build guard or CI: answer "is refs.bib current?" and
///   write nothing at all.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RefsMode {
    Sync,
    Generate,
    Check,
}

fn run_refs(
    lib: &mut MergedLibrary,
    root: &std::path::Path,
    file: Option<&std::path::Path>,
    output: Option<&std::path::Path>,
    prune: bool,
    dry_run: bool,
    mode: RefsMode,
) -> anyhow::Result<()> {
    use astrobib::export;
    use astrobib::library::CiteState;

    let tex_files = export::manuscript_tex_files(root);
    let md_files = export::manuscript_md_files(root);
    let mut cited: Vec<String> = export::scan_tex_files(&tex_files);
    let mut seen: std::collections::HashSet<String> = cited.iter().cloned().collect();
    for c in export::scan_md_files(&md_files) {
        if (!c.wikilink || lib.resolve_citation(&c.raw).1.is_some()) && seen.insert(c.raw.clone())
        {
            cited.push(c.raw);
        }
    }

    // sync: copy library-only cites into the manuscript db
    let mut copied: Vec<String> = vec![];
    let mut uncopied: Vec<String> = vec![];
    let mut missing: Vec<String> = vec![];
    let mut ambiguous: Vec<String> = vec![];
    let mut targets: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut sorted = cited.clone();
    sorted.sort();
    for c in &sorted {
        let (state, entry) = lib.resolve_citation(c);
        let key = entry.map(|e| e.key().to_string());
        match state {
            CiteState::Ok => {
                targets.insert(key.unwrap());
            }
            CiteState::Library => {
                let key = key.unwrap();
                // Generate and Check never modify bib/: a build step
                // must not write into its own inputs
                if !dry_run && mode == RefsMode::Sync {
                    lib.add_to_manuscript(&key)?;
                    copied.push(if *c == key { key.clone() } else { format!("{c} → {key}") });
                    targets.insert(key);
                } else {
                    uncopied.push(if *c == key { key.clone() } else { format!("{c} → {key}") });
                }
            }
            CiteState::Ambiguous => ambiguous.push(c.clone()),
            CiteState::Missing => missing.push(c.clone()),
        }
    }
    let uncited: Vec<String> = lib
        .manuscript
        .as_ref()
        .map(|ms| {
            ms.entries()
                .iter()
                .map(|e| e.key().to_string())
                .filter(|k| !targets.contains(k))
                .collect()
        })
        .unwrap_or_default();
    if prune && !dry_run && mode == RefsMode::Sync {
        for k in &uncited {
            lib.remove_from_manuscript(k)?; // sole copies are rescued
        }
    }

    // Copying an entry into bib/ writes it through a parse→serialise
    // cycle, which flips its trailing fields (the documented two-form
    // oscillation). Generating refs.bib from the in-memory copy would
    // therefore disagree with a fresh read of the same file — so
    // `refs` would not be idempotent, and `refs --check` would report
    // "out of date" immediately after a successful sync. Re-read from
    // disk so one run settles.
    if mode == RefsMode::Sync && !copied.is_empty() && !dry_run {
        let global_on = lib.global_on;
        *lib = MergedLibrary::load(Some(root))?;
        lib.global_on = global_on;
    }

    // refs.bib from the manuscript db, keyed by the cited strings
    let bib_out = match output {
        Some(p) if p.is_absolute() => Some(p.to_path_buf()),
        Some(p) => Some(root.join(p)),
        None if !tex_files.is_empty() || root.join("refs.bib").exists() => {
            Some(root.join("refs.bib"))
        }
        None => None, // pure-markdown manuscript: no refs.bib unless asked
    };
    let content = export::refs_bib_content(&cited, lib);
    let n_bib = {
        let mut s = cited.clone();
        s.sort();
        s.dedup();
        s.iter()
            .filter(|c| matches!(lib.resolve_citation(c).0, CiteState::Ok))
            .count()
    };
    if let Some(out) = &bib_out {
        // never truncate a bibliography that is already there: when no
        // cite resolves (an unadopted manuscript, a library not yet
        // synced) the file on disk is the user's own — hand-written or
        // from another tool — and overwriting it with nothing loses
        // bibdata astrobib cannot regenerate
        let occupied = std::fs::read_to_string(out).is_ok_and(|t| !t.trim().is_empty());
        if content.trim().is_empty() && occupied {
            eprintln!(
                "refusing to overwrite {} with an empty bibliography — \
                 no cited key resolves in the library.",
                out.display()
            );
        } else if mode == RefsMode::Check {
            // The question a build guard is really asking is "would
            // running astrobib refs change anything?" — so an entry that
            // a sync would copy into bib/ counts as stale too, and an
            // absent file is stale even when the content would be empty
            // (comparing "" to a missing file would call it current).
            let why = match std::fs::read_to_string(out) {
                Err(_) => Some("missing".to_string()),
                Ok(disk) if disk != content => Some("out of date".to_string()),
                _ if !uncopied.is_empty() => Some(format!(
                    "out of sync ({} cited entr{} not in bib/)",
                    uncopied.len(),
                    if uncopied.len() == 1 { "y is" } else { "ies are" }
                )),
                _ => None,
            };
            match why {
                None => println!("{} is current ({n_bib} entries)", out.display()),
                Some(why) => {
                    eprintln!("{} is {why} — run: astrobib refs", out.display());
                    std::process::exit(1);
                }
            }
        } else if dry_run {
            println!("would write {n_bib} entr{} → {}", if n_bib == 1 { "y" } else { "ies" }, out.display());
        } else {
            let changed = export::write_refs_bib(out, &content)?;
            // A build system judges freshness by mtime, so a CLI run
            // always stamps the file even when the bytes are identical:
            // otherwise the target stays older than its prerequisites
            // and make re-runs the rule on every single build. The TUI's
            // background sync deliberately does NOT do this — it runs
            // constantly, and advancing the mtime there would provoke a
            // full LaTeX rebuild for nothing.
            if !changed {
                stamp(out)?;
            }
            println!(
                "{n_bib} entr{} → {}{}",
                if n_bib == 1 { "y" } else { "ies" },
                out.display(),
                if changed { "" } else { "  (unchanged)" }
            );
        }
    }

    if mode == RefsMode::Check {
        // any markdown bibliography is checked the same way, then done
        return check_markdown(lib, root, file, &md_files, &cited);
    }

    // rendered bibliography for a markdown manuscript
    if !md_files.is_empty() {
        let target = match file {
            Some(f) => Some(f.to_path_buf()),
            None => match md_files.first() {
                Some(f) if md_files.len() == 1 || f.ends_with("main.md") => Some(f.clone()),
                _ => {
                    eprintln!("Several .md files and no main.md — name the target: astrobib refs FILE");
                    None
                }
            },
        };
        if let Some(target) = target {
            let mut keys_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut entries = vec![];
            for c in &cited {
                if let Some(e) = lib.resolve_citation(c).1 {
                    if keys_seen.insert(e.key().to_string()) {
                        entries.push(e);
                    }
                }
            }
            let block = export::render_md_bibliography(&entries);
            if dry_run {
                println!("{block}");
            } else {
                let changed = export::update_md_bibliography(&target, &block)?;
                println!(
                    "{} reference(s) in {}{}",
                    entries.len(),
                    target.display(),
                    if changed { "" } else { "  (unchanged)" }
                );
            }
        }
    }

    if !uncopied.is_empty() {
        println!(
            "\n{} cited entr{} in the global library but not in bib/ (astrobib refs copies them in):",
            uncopied.len(),
            if uncopied.len() == 1 { "y is" } else { "ies are" }
        );
        for k in &uncopied {
            println!("  {k}");
        }
    }
    if !copied.is_empty() {
        println!(
            "{}opied {} entr{} from the personal library:",
            if dry_run { "would have c" } else { "C" },
            copied.len(),
            if copied.len() == 1 { "y" } else { "ies" }
        );
        for k in &copied {
            println!("  {k}");
        }
    }
    if prune && !uncited.is_empty() {
        println!(
            "{} {} uncited entr{}",
            if dry_run { "would prune" } else { "Pruned" },
            uncited.len(),
            if uncited.len() == 1 { "y" } else { "ies" }
        );
    } else if !uncited.is_empty() {
        println!("{} uncited entr{} in the manuscript db (--prune removes)", uncited.len(), if uncited.len() == 1 { "y" } else { "ies" });
    }
    for c in &ambiguous {
        eprintln!("ambiguous: {c}");
    }
    for c in &missing {
        eprintln!("missing: {c}");
    }
    Ok(())
}

/// Which tiers a write lands in, local-first: the local library when
/// there is one, and the global library only when it is asked for — or
/// when it is all there is. The rule every write path shares, so `add`
/// and `import` cannot disagree about where an unflagged run goes.
fn write_tiers(lib: &MergedLibrary, share: bool) -> (bool, bool) {
    let local = lib.manuscript.is_some();
    (share || !local, local)
}

/// What to call a (global, local) destination in a report.
fn tier_label(to_global: bool, to_local: bool) -> &'static str {
    match (to_global, to_local) {
        (true, true) => "global + local libraries",
        (true, false) => "global library",
        _ => "local library",
    }
}

/// The flags one import run was given. They are read in four different
/// places — the preflight, the destination label, the write pass and the
/// report — which is more than a signature should carry positionally.
struct ImportOpts {
    global: bool,
    global_only: bool,
    local_only: bool,
    cited_only: bool,
    rename_citekeys: bool,
    dry_run: bool,
}

/// What the import would do with one entry of the file. The resolution
/// pass decides it, the write pass acts on it, and the report describes
/// it — a split that exists because resolution is a *read*: `--dry-run`
/// runs the first pass and the last, and skips the one in between.
enum Fate {
    /// A record the library does not hold. This is the import.
    New { orig: String, key: String, data: astrobib::bib::Data },
    /// Already held, under `key` — which need not be the key the file
    /// gave it, and when it is not, that is a rename the sources need.
    /// `aliased` marks the match that was made by bibcode against a
    /// *different* canonical key, usually a preprint meeting its
    /// published record: worth saying, because the entry the library
    /// keeps is then not the one the file described.
    Present { orig: String, key: String, aliased: bool },
    /// ADS could not settle what the entry is.
    Failed { orig: String, reason: String },
}

impl Fate {
    /// The library key this entry ends up denoting, if any.
    fn key(&self) -> Option<&str> {
        match self {
            Fate::New { key, .. } | Fate::Present { key, .. } => Some(key),
            Fate::Failed { .. } => None,
        }
    }
}

/// astrobib import — bring a foreign .bib file in. Entries whose
/// cite key is reproducible from their own data are canonical astrobib
/// bibdata and import directly; everything else resolves against ADS
/// (arXiv ID → DOI → exact title+author+year, which must be unique).
/// Already-present entries are kept; renames print as an old→new map.
/// With `cited_only`, the manuscript's own cites decide what comes in.
fn import_bib(
    lib: &mut MergedLibrary,
    file: &std::path::Path,
    local_root: Option<&std::path::Path>,
    opts: ImportOpts,
) -> anyhow::Result<()> {
    let ImportOpts { global, global_only, local_only, cited_only, rename_citekeys, dry_run } = opts;
    // A tier flag naming a tier that is not there is a typo, and it is
    // one before any entry is read: the write pass would otherwise print
    // a destination it cannot honour and then fail on the first entry.
    if local_only && lib.manuscript.is_none() {
        anyhow::bail!("--local-only requires a local (tier-2) library — none found here.");
    }
    // Checked before a single entry is written: the rewrite is the point
    // of the flag, and discovering there was nowhere to apply it after
    // the import would leave the map on the terminal and nothing to do
    // with it — which is the state the flag exists to end.
    let rename_root = if rename_citekeys {
        let Some(root) = local_root else {
            anyhow::bail!(
                "--rename-citekeys needs a manuscript directory — none found here."
            );
        };
        if !astrobib::export::has_manuscript_sources(root) {
            anyhow::bail!(
                "no .tex or .md sources in {} — nothing to rewrite cite keys in.",
                root.display()
            );
        }
        Some(root.to_path_buf())
    } else {
        None
    };
    let text = std::fs::read_to_string(file)?;
    let entries = astrobib::bib::parse_entries(&text);
    if entries.is_empty() {
        println!("No entries found in file.");
        return Ok(());
    }
    // A coauthor's refs.bib is usually their whole collection rather
    // than this paper's bibliography, so the manuscript's cites are the
    // only statement of which part of it belongs here. The filter runs
    // before ADS resolution, so what is left out costs nothing — which
    // is the point when the file holds hundreds of entries and the
    // paper cites thirty.
    let cited: Option<Vec<String>> = if cited_only {
        let Some(root) = local_root else {
            anyhow::bail!("--cited-only needs a manuscript directory — none found here.");
        };
        let tex = astrobib::export::manuscript_tex_files(root);
        let md = astrobib::export::manuscript_md_files(root);
        // No sources means no cites, and every entry would be skipped:
        // a silent no-op is indistinguishable from a foreign file that
        // happens to share nothing with the paper, so say so instead.
        if tex.is_empty() && md.is_empty() {
            anyhow::bail!(
                "no .tex or .md sources in {} — nothing to match cites against.",
                root.display()
            );
        }
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut keys: Vec<String> = vec![];
        // wikilinks count here even when they do not resolve, unlike in
        // refs: the entries they would resolve to are precisely the ones
        // this import is about to add
        for c in astrobib::export::scan_tex_files(&tex)
            .into_iter()
            .chain(astrobib::export::scan_md_files(&md).into_iter().map(|c| c.raw))
        {
            if seen.insert(c.clone()) {
                keys.push(c);
            }
        }
        Some(keys)
    } else {
        None
    };
    // Which tiers the write pass would touch — and so, exactly, which
    // key sets the short keys have to be computed against. Derived once
    // rather than re-read from the flags at each use.
    //
    // `--global` is not gated by the `--no-global` display switch: one
    // says which tier this import means, the other says which tier is on
    // screen, and the flag naming a tier outright is the more specific
    // statement of the two.
    let (to_global, to_local) = if global_only {
        (true, false)
    } else if local_only {
        // the preflight above established there is a local tier
        (false, true)
    } else {
        write_tiers(lib, global)
    };
    let dest = tier_label(to_global, to_local);
    let mut head = if dry_run {
        format!("Dry run — nothing will be written.\nWould import into {dest}")
    } else {
        format!("Importing into {dest}")
    };
    if let Some(c) = &cited {
        head += &format!(
            " — only what the manuscript cites ({} cite{} scanned)",
            c.len(),
            if c.len() == 1 { "" } else { "s" }
        );
    }
    if dry_run {
        // which tier is which directory, said once, so the per-entry
        // lines below need only name the file
        if to_global {
            head += &format!("\n  global  {}", lib.personal.root.join("bib").display());
        }
        if to_local {
            if let Some(ms) = &lib.manuscript {
                head += &format!("\n  local   {}", ms.root.join("bib").display());
            }
        }
    }
    println!("{head}\n");
    let (kept, left_out): (Vec<astrobib::bib::Data>, Vec<_>) =
        entries.into_iter().partition(|d| match &cited {
            Some(c) => entry_is_cited(d.get("ID").map(String::as_str).unwrap_or("?"), d, c),
            None => true,
        });
    let uncited = left_out.len();
    // A canonical entry needs no lookup; the rest each cost a round trip,
    // and the count is the only warning that this will take a while.
    let to_resolve = kept
        .iter()
        .filter(|d| d.get("ID").map(String::as_str) != Some(&astrobib::keys::generate_key(d)))
        .count();
    if to_resolve > 0 {
        println!(
            "Resolving {to_resolve} entr{} against ADS…\n",
            if to_resolve == 1 { "y" } else { "ies" }
        );
    }

    // ── resolution: reads only, so a dry run gets this far too ──
    let mut fates: Vec<Fate> = vec![];
    // keys and bibcodes this run has already claimed. A file may name one
    // paper twice, and the second copy is already present whether or not
    // the write pass has run yet — which is the only way the two passes
    // can agree about what the library holds.
    let mut pending: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pending_bibcodes: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for mut data in kept {
        let orig = data.get("ID").cloned().unwrap_or_else(|| "?".to_string());
        if orig != astrobib::keys::generate_key(&data) {
            match astrobib::ads::lookup_entry(&data) {
                Ok(resolved) => data = resolved,
                Err(reason) => {
                    fates.push(Fate::Failed { orig, reason });
                    continue;
                }
            }
        }
        let key = astrobib::keys::generate_key(&data);
        // dedupe by bibcode: catches the same paper under a different key
        let bc = data
            .get("adsurl")
            .and_then(|u| astrobib::pdf::bibcode_from_adsurl(u))
            .map(str::to_string);
        let by_bibcode = bc.as_deref().and_then(|b| {
            lib.get_by_bibcode(b)
                .map(|e| e.key().to_string())
                .or_else(|| pending_bibcodes.get(b).cloned())
        });
        if let Some(existing) = by_bibcode {
            if existing != key {
                fates.push(Fate::Present { orig, key: existing, aliased: true });
                continue;
            }
        }
        if lib.get(&key).is_some() || pending.contains(&key) {
            fates.push(Fate::Present { orig, key, aliased: false });
            continue;
        }
        pending.insert(key.clone());
        if let Some(b) = bc {
            pending_bibcodes.insert(b, key.clone());
        }
        fates.push(Fate::New { orig, key, data });
    }

    // ── the write pass, and the only one of the three that writes ──
    if !dry_run {
        for f in &fates {
            let Fate::New { data, .. } = f else { continue };
            // the tiers decided above, not the flags that decided them:
            // `to_local` is true only where there is a local tier to
            // write, which is what makes the unwrap sound
            if to_local {
                lib.manuscript.as_mut().unwrap().save_entry(data)?;
            }
            if to_global {
                lib.personal.save_entry(data)?;
            }
        }
    }

    // ── the report, over the library as it now stands (or would) ──
    //
    // Short keys are read *after* the whole batch rather than as each
    // entry lands: the shortest unambiguous prefix is a function of the
    // set, so an entry reported early can be shortened by a later one in
    // the same file. That is why the map printed here is also the map
    // applied to the sources below — both name the library that exists
    // when the import is over.
    let reported: Vec<String> = fates.iter().filter_map(|f| f.key().map(str::to_string)).collect();
    let shorts = if dry_run {
        previewed_short_keys(lib, &pending, &reported, to_global, to_local)
    } else {
        reported
            .iter()
            .map(|k| {
                let short = lib
                    .get(k)
                    .map(|e| if e.short_key.is_empty() { k.clone() } else { e.short_key.clone() })
                    .unwrap_or_else(|| k.clone());
                (k.clone(), short)
            })
            .collect()
    };
    let short_of = |k: &str| shorts.get(k).cloned().unwrap_or_else(|| k.to_string());
    let (mut added, mut skipped) = (0usize, 0usize);
    let mut renames: Vec<(String, String)> = vec![];
    for f in &fates {
        match f {
            Fate::Failed { orig, reason } => {
                println!("  ⚠ {orig} skipped — {reason}");
                skipped += 1;
            }
            Fate::Present { orig, key, aliased } => {
                let short = short_of(key);
                let note = if *aliased {
                    "already present under a different key — kept existing"
                } else {
                    "already present — kept existing"
                };
                if *orig != short {
                    println!("  {orig} → {short}  ({note})");
                    renames.push((orig.clone(), short));
                } else {
                    println!("  {short}  ({note})");
                }
                skipped += 1;
            }
            Fate::New { orig, key, .. } => {
                let short = short_of(key);
                let note = if dry_run { format!("  (would write {key}.bib)") } else { String::new() };
                if *orig != short {
                    println!("  {orig} → {short}{note}");
                    renames.push((orig.clone(), short));
                } else {
                    println!("  {short}{note}");
                }
                added += 1;
            }
        }
    }
    if dry_run {
        println!("\n{added} would be imported → {dest}, {skipped} skipped.");
    } else {
        println!("\n{added} imported → {dest}, {skipped} skipped.");
    }
    if cited.is_some() {
        println!(
            "{uncited} entr{} left out — not cited by the manuscript.",
            if uncited == 1 { "y" } else { "ies" }
        );
    }
    if !renames.is_empty() {
        match &rename_root {
            Some(_) => println!("\nRe-keyed:"),
            None => println!(
                "\nRe-keyed (old cites resolve by prefix or bibcode; others show as missing):"
            ),
        }
        for (old, new) in &renames {
            println!("  {old} → {new}");
        }
    }
    // The rewrite runs here or not at all: `convert` derives its map from
    // `resolve_citation`, which knows nothing of a foreign key, so the
    // mapping is unrecoverable once this process exits. It prints per
    // file, because this is the one thing astrobib writes to files it
    // does not own — and under --dry-run it counts without writing, which
    // is the same code deciding the same edits.
    if let Some(root) = rename_root {
        if renames.is_empty() {
            let what = if dry_run { "would be imported" } else { "imported" };
            println!("\nNothing to rename — every key {what} under the one it already had.");
        } else {
            let verb = if dry_run { "Would rewrite" } else { "Rewriting" };
            println!("\n{verb} cite keys in {}:", root.display());
            let n = rewrite_citations(&root, &renames, dry_run);
            if n == 0 {
                println!("  no cites matched — the sources never used these keys.");
            } else if !dry_run {
                // refs.bib is not regenerated here: an import may have
                // been told to write only one tier, and refs syncs the
                // manuscript db as it goes — that is a decision for the
                // next command rather than a side effect of this one
                println!("  refs.bib now names the old keys — run astrobib refs.");
            }
        }
    }
    if dry_run {
        println!("\nNothing was written. Re-run without --dry-run to import.");
    }
    Ok(())
}

/// The short keys the library *would* report once this import is done,
/// computed without writing anything.
///
/// A rename's right-hand side is the shortest unambiguous prefix, and
/// that is a function of the library as it stands after the import: two
/// papers by one author and year shorten differently once both are in.
/// So a preview cannot ask the library what a key would shorten to — it
/// shortens against each tier's keys plus the entries the run would add
/// to that tier, which is also why the whole batch is resolved before
/// any of it is reported.
///
/// Which tier answers for a key is `MergedLibrary::get`'s rule, and has
/// to be: the short key the real run prints is the one the entry it
/// finds carries. An entry already held globally keeps its global short
/// key through a `--local-only` run, pending entries and all.
fn previewed_short_keys(
    lib: &MergedLibrary,
    pending: &std::collections::HashSet<String>,
    reported: &[String],
    to_global: bool,
    to_local: bool,
) -> std::collections::HashMap<String, String> {
    let tier = |keys: Vec<String>, gains: bool| -> Vec<String> {
        let mut v = keys;
        if gains {
            v.extend(pending.iter().cloned());
        }
        v.sort();
        v.dedup();
        v
    };
    let global = tier(
        lib.personal.entries().iter().map(|e| e.key().to_string()).collect(),
        to_global,
    );
    let local = tier(
        lib.manuscript
            .as_ref()
            .map(|m| m.entries().iter().map(|e| e.key().to_string()).collect())
            .unwrap_or_default(),
        to_local,
    );
    reported
        .iter()
        .map(|k| {
            let set = if lib.global_active() && global.binary_search(k).is_ok() {
                &global
            } else {
                &local
            };
            (k.clone(), astrobib::keys::shortest_unambiguous(k, set))
        })
        .collect()
}

/// Does a foreign entry answer any cite in the manuscript? Exact key
/// first, then astrobib's own prefix rule — a cite may be any prefix of
/// a key, which is how a short key cites a full one — and finally the
/// bibcode, citable in its own right. Matching is on the entry's *own*
/// key, the one the manuscript was written against; the canonical key
/// it will be imported under is not in the sources yet.
fn entry_is_cited(orig_key: &str, data: &astrobib::bib::Data, cited: &[String]) -> bool {
    let bibcode = data.get("adsurl").and_then(|u| astrobib::pdf::bibcode_from_adsurl(u));
    cited
        .iter()
        .any(|c| orig_key.starts_with(c.as_str()) || bibcode == Some(c.as_str()))
}

fn print_entries(
    lib: &MergedLibrary,
    pred: impl Fn(&astrobib::library::Entry) -> bool,
    limit: usize,
) {
    let mut shown: Vec<_> = lib.entries().into_iter().filter(|e| pred(e)).collect();
    shown.sort_by(|a, b| b.year().cmp(&a.year()).then(a.key().cmp(b.key())));
    for e in shown.iter().take(limit) {
        println!(
            "{:<24} {:<6} {:<18} {}",
            e.short_key,
            e.year(),
            astrobib::text::fit_authors(e.author(), 18),
            truncate(e.title().trim_matches(['{', '}']), 60),
        );
    }
    println!("{} result(s)", shown.len().min(limit));
}

fn truncate(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}
