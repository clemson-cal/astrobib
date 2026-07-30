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
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Search the local library, or ADS with --ads
    Search {
        query: String,
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        ads: bool,
    },
    /// Add a paper by ADS bibcode (or pasted ADS URL)
    Add {
        bibcode: String,
        #[arg(short, long)]
        force: bool,
    },
    /// Print the BibTeX entry for a cite key (full or shortened)
    Show { key: String },
    /// Show the resolved environment, or set a value (token, email)
    Config {
        /// Field to set: ads_token | email
        #[arg(value_parser = ["ads_token", "email"], requires = "value")]
        key: Option<String>,
        /// New value for the field
        value: Option<String>,
    },
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
        file: std::path::PathBuf,
        /// Write only to the global (tier-1) library
        #[arg(long)]
        global_only: bool,
        /// Write only to the local (tier-2) library
        #[arg(long)]
        local_only: bool,
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
        #[arg(long)]
        prune: bool,
        /// refs.bib output path (default: refs.bib in the manuscript root)
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(p) = &cli.library {
        // one resolution path: the flag wins by shadowing the env var
        // (read before any thread starts)
        std::env::set_var("ASTROBIB_LIBRARY", p);
    }
    let ms_root = match &cli.path {
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
    let mut lib = MergedLibrary::load(ms_root.as_deref())?;
    astrobib::tui::debug_startup(&format!(
        "library load {:?} ms={}",
        t_load.elapsed(),
        ms_root.is_some()
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
            let ctx = QueryContext::default();
            print_entries(&lib, |e| query::matches(&groups, e, &ctx), limit);
            Ok(())
        }
        Some(Command::Add { bibcode, force }) => {
            let bc = astrobib::ads::bibcode_from_url(&bibcode).unwrap_or(bibcode);
            let Some(data) = astrobib::ads::fetch_bibtex(&bc)? else {
                eprintln!("Could not fetch BibTeX for {bc}");
                std::process::exit(1);
            };
            let key = astrobib::keys::generate_key(&data);
            if lib.personal.has(&key) && !force {
                eprintln!("{key} already in library. Use --force to overwrite.");
                std::process::exit(1);
            }
            let key = lib.save_entry(&data)?;
            let e = lib.get(&key).unwrap();
            let display = if e.short_key.is_empty() { &key } else { &e.short_key };
            println!("Added {display}  ({key})");
            Ok(())
        }
        Some(Command::Import { file, global_only, local_only }) => {
            import_bib(&mut lib, &file, global_only, local_only)
        }
        Some(Command::Tidy { dry_run }) => {
            match ms_root.clone() {
                Some(root) => tidy_bib_dir(&mut lib, &root, dry_run),
                // no bib/ anywhere: a legacy manuscript (sources plus
                // loose .bib files) can be adopted wholesale
                None => adopt_manuscript(&mut lib, dry_run),
            }
        }
        Some(Command::Refs { file, dry_run, prune, output }) => {
            let Some(root) = ms_root.clone() else {
                eprintln!("No local bib root here — refs needs a manuscript directory.");
                std::process::exit(1);
            };
            run_refs(&mut lib, &root, file.as_deref(), output.as_deref(), prune, dry_run)
        }
        Some(Command::Update { all }) => run_update(&mut lib, all),
        Some(Command::Convert { format, dry_run }) => {
            let Some(root) = ms_root.clone() else {
                eprintln!("No local bib root here — convert needs a manuscript directory.");
                std::process::exit(1);
            };
            run_convert(&mut lib, &root, &format, dry_run)
        }
        Some(Command::Config { key: Some(key), value: Some(value) }) => {
            astrobib::ads::save_state_field(&key, &value)?;
            println!("{key} saved.");
            Ok(())
        }
        Some(Command::Config { .. }) => {
            run_config(&lib, ms_root.as_deref(), cli.library.is_some());
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
    if let Some(q) = ads::get_quota() {
        summary.push_str(&format!("  · ADS quota {}/{}", q.remaining, q.limit));
    }
    println!("{summary}");
    Ok(())
}

/// astrobib config — the resolved environment, for humans debugging it.
fn run_config(lib: &MergedLibrary, ms_root: Option<&std::path::Path>, via_flag: bool) {
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
    match (ms_root, &lib.manuscript) {
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
    if let Some(q) = astrobib::ads::get_quota() {
        println!("ADS quota        {}/{} remaining", q.remaining, q.limit);
    }
    let cache = astrobib::library::pdf_cache_dir();
    let (n, bytes) = std::fs::read_dir(&cache)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok())
                .fold((0usize, 0u64), |(n, b), m| (n + 1, b + m.len()))
        })
        .unwrap_or((0, 0));
    println!(
        "PDF cache        {}  ({n} files, {:.1} MB)",
        cache.display(),
        bytes as f64 / 1e6
    );
}

/// Rewrite old→new cite keys inside every manuscript source's \cite
/// braces, reporting per file.
fn rewrite_citations(root: &std::path::Path, renames: &[(String, String)]) {
    use std::collections::HashMap;
    let map: HashMap<&str, &str> =
        renames.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut total = 0usize;
    for f in astrobib::export::manuscript_tex_files(root) {
        match astrobib::export::convert_citations(&f, |k| map.get(k).map(|s| s.to_string())) {
            Ok(0) => {}
            Ok(n) => {
                total += n;
                println!("  rewrote {n} cite(s) in {}", f.display());
            }
            Err(e) => eprintln!("  could not rewrite {}: {e}", f.display()),
        }
    }
    let _ = total;
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
        rewrite_citations(root, &renames);
        println!("{} cite key(s) converted to {format} form", renames.len());
        run_refs(lib, root, None, None, false, false)?;
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
        rewrite_citations(&root, &renames);
    }
    println!(
        "\n{adopted} entr{} adopted, {skipped} left alone.",
        if adopted == 1 { "y" } else { "ies" }
    );
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
    run_refs(lib, &root, None, None, false, false)
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
    if foreign.is_empty() {
        println!("bib/ is already canonical.");
        return run_refs(lib, root, None, None, false, dry_run);
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
        rewrite_citations(root, &renames);
    }
    println!();
    run_refs(lib, root, None, None, false, false)
}

/// astrobib refs — the manuscript sync flow, for TeX and markdown
/// manuscripts alike. Cited keys missing from the manuscript db are
/// copied in from the personal library; keys found nowhere are reported;
/// refs.bib is generated from the manuscript db (each entry keyed by the
/// string actually cited); a markdown manuscript also gets its rendered
/// bibliography.
fn run_refs(
    lib: &mut MergedLibrary,
    root: &std::path::Path,
    file: Option<&std::path::Path>,
    output: Option<&std::path::Path>,
    prune: bool,
    dry_run: bool,
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
                if !dry_run {
                    lib.add_to_manuscript(&key)?;
                }
                copied.push(if *c == key { key.clone() } else { format!("{c} → {key}") });
                targets.insert(key);
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
    if prune && !dry_run {
        for k in &uncited {
            lib.remove_from_manuscript(k)?; // sole copies are rescued
        }
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
        if dry_run {
            println!("would write {n_bib} entr{} → {}", if n_bib == 1 { "y" } else { "ies" }, out.display());
        } else {
            let changed = export::write_refs_bib(out, &content)?;
            println!(
                "{n_bib} entr{} → {}{}",
                if n_bib == 1 { "y" } else { "ies" },
                out.display(),
                if changed { "" } else { "  (unchanged)" }
            );
        }
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

/// astrobib import — bring a foreign .bib file in. Entries whose
/// cite key is reproducible from their own data are canonical astrobib
/// bibdata and import directly; everything else resolves against ADS
/// (arXiv ID → DOI → exact title+author+year, which must be unique).
/// Already-present entries are kept; renames print as perl one-liners.
fn import_bib(
    lib: &mut MergedLibrary,
    file: &std::path::Path,
    global_only: bool,
    local_only: bool,
) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(file)?;
    let entries = astrobib::bib::parse_entries(&text);
    if entries.is_empty() {
        println!("No entries found in file.");
        return Ok(());
    }
    let dest = match (global_only, local_only, lib.manuscript.is_some()) {
        (true, _, _) | (_, _, false) => "global library".to_string(),
        (_, true, _) => "local library".to_string(),
        _ => "global + local libraries".to_string(),
    };
    println!("Importing into {dest}\n");
    let (mut added, mut skipped) = (0usize, 0usize);
    let mut renames: Vec<(String, String)> = vec![];
    for mut data in entries {
        let orig_key = data.get("ID").cloned().unwrap_or_else(|| "?".to_string());
        if orig_key != astrobib::keys::generate_key(&data) {
            match astrobib::ads::lookup_entry(&data) {
                Ok(resolved) => data = resolved,
                Err(reason) => {
                    println!("  ⚠ {orig_key} skipped — {reason}");
                    skipped += 1;
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
        if let Some(existing) = bc.as_deref().and_then(|b| lib.get_by_bibcode(b)) {
            if existing.key() != key {
                let short = if existing.short_key.is_empty() {
                    existing.key().to_string()
                } else {
                    existing.short_key.clone()
                };
                println!("  {orig_key} → {short}  (already present under a different key — kept existing)");
                if orig_key != short {
                    renames.push((orig_key, short));
                }
                skipped += 1;
                continue;
            }
        }
        if lib.get(&key).is_some() {
            let short = lib
                .get(&key)
                .map(|e| if e.short_key.is_empty() { key.clone() } else { e.short_key.clone() })
                .unwrap_or_else(|| key.clone());
            println!("  {orig_key} → {short}  (already present — kept existing)");
            if orig_key != short {
                renames.push((orig_key, short));
            }
            skipped += 1;
            continue;
        }
        let new_key = if global_only {
            lib.personal.save_entry(&data)?
        } else if local_only {
            match &mut lib.manuscript {
                Some(ms) => ms.save_entry(&data)?,
                None => anyhow::bail!("--local-only requires a local (tier-2) library"),
            }
        } else {
            lib.save_entry(&data)?
        };
        let short = lib
            .get(&new_key)
            .map(|e| if e.short_key.is_empty() { new_key.clone() } else { e.short_key.clone() })
            .unwrap_or_else(|| new_key.clone());
        if orig_key != short {
            renames.push((orig_key.clone(), short.clone()));
            println!("  {orig_key} → {short}");
        } else {
            println!("  {short}");
        }
        added += 1;
    }
    println!("\n{added} imported → {dest}, {skipped} skipped.");
    if !renames.is_empty() {
        println!("\nRe-keyed (old cites resolve by prefix or bibcode; others show as missing):");
        for (old, new) in renames {
            println!("  {old} → {new}");
        }
    }
    Ok(())
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
