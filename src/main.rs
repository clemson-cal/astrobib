use astrobib::library::{find_manuscript_db, MergedLibrary};
use astrobib::query::{self, QueryContext};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "astrobib", version, about = "ADS-native BibTeX library manager (Rust port)")]
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
            let p = p.canonicalize().unwrap_or_else(|_| p.clone());
            Some(p) // explicit tier-2 root; bib/ is created lazily on write
        }
        None => find_manuscript_db(),
    };
    let mut lib = MergedLibrary::load(ms_root.as_deref())?;
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
            let Some(root) = ms_root.clone() else {
                eprintln!("No local bib root here — tidy needs a manuscript directory.");
                std::process::exit(1);
            };
            tidy_bib_dir(&mut lib, &root, dry_run)
        }
        Some(Command::Refs { file, dry_run, prune, output }) => {
            let Some(root) = ms_root.clone() else {
                eprintln!("No local bib root here — refs needs a manuscript directory.");
                std::process::exit(1);
            };
            run_refs(&mut lib, &root, file.as_deref(), output.as_deref(), prune, dry_run)
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
        println!("\nCite key replacements (copy/paste to update the manuscript):");
        for (old, new) in &renames {
            println!("  perl -pi -e 's/\\b\\Q{old}\\E\\b/{new}/g' main.tex");
        }
    }
    println!();
    run_refs(lib, root, None, None, false, false)
}

/// astrobib refs — port of the v0.4.0 manuscript sync flow, extended to
/// markdown manuscripts. Cited keys missing from the manuscript db are
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

/// Import a .bib file — port of the v0.4.0 import command. Entries whose
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
        println!("\nCite key replacements (copy/paste to update your TeX source):");
        for (old, new) in renames {
            println!("  perl -pi -e 's/\\b\\Q{old}\\E\\b/{new}/g' main.tex");
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
