//! CLI integration tests: the compiled binary driven as a subprocess
//! against a scratch sandbox built from tests/tui/fixtures/.
//!
//! Sandboxing: HOME, ASTROBIB_LIBRARY and ASTROBIB_STATE_DIR all point
//! into a per-test temp tree, and the child environment is cleared
//! outright (only PATH survives), so no invocation can ever read or
//! write the user's real library, state, or PDF cache — the same rule
//! tests/tui/driver.py follows. Clearing the environment also removes
//! ADS_API_TOKEN, so nothing here can reach the network by accident.
//!
//! Network: tests that need ADS are skipped unless RUN_ADS_TESTS=1
//! (with ADS_API_TOKEN set), mirroring tests/tui/run.py.

use astrobib::bib::{self, Data};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_astrobib");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/tui/fixtures");

static SEQ: AtomicUsize = AtomicUsize::new(0);

// ── sandbox ─────────────────────────────────────────────────────────

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    library: PathBuf,
}

impl Sandbox {
    /// A scratch tree with the fixture library already in place.
    fn new(tag: &str) -> Sandbox {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir()
            .join(format!("astrobib-cli-{}-{n}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("state")).unwrap();
        std::fs::create_dir_all(root.join("library").join("bib")).unwrap();
        // canonical so $HOME compares equal to the child's cwd (macOS
        // resolves /tmp through /private)
        let root = root.canonicalize().unwrap();
        let sb = Sandbox {
            home: root.join("home"),
            state: root.join("state"),
            library: root.join("library"),
            root,
        };
        for f in std::fs::read_dir(FIXTURES).unwrap().flatten() {
            let p = f.path();
            if p.extension().is_some_and(|x| x == "bib") {
                std::fs::copy(&p, sb.bib_dir().join(p.file_name().unwrap())).unwrap();
            }
        }
        sb
    }

    /// The same tree with an empty library (no fixtures).
    fn empty(tag: &str) -> Sandbox {
        let sb = Sandbox::new(tag);
        for f in std::fs::read_dir(sb.bib_dir()).unwrap().flatten() {
            std::fs::remove_file(f.path()).unwrap();
        }
        sb
    }

    fn bib_dir(&self) -> PathBuf {
        self.library.join("bib")
    }

    /// A manuscript root under $HOME, with a bib/ so the walk-up finds it.
    fn ms(&self, name: &str) -> PathBuf {
        let dir = self.home.join(name);
        std::fs::create_dir_all(dir.join("bib")).unwrap();
        dir
    }

    /// A plain directory under $HOME (no bib/ — nothing to walk up to).
    fn dir(&self, name: &str) -> PathBuf {
        let dir = self.home.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pdf_cache(&self) -> PathBuf {
        self.home.join(".cache/astrobib/pdfs")
    }

    fn query_cache(&self) -> PathBuf {
        self.home.join(".cache/astrobib/query_cache.json")
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_in(&self.home, args)
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Run {
        self.run_env(cwd, args, &[])
    }

    fn run_env(&self, cwd: &Path, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let mut cmd = Command::new(BIN);
        cmd.env_clear()
            .env(
                "PATH",
                std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()),
            )
            .env("HOME", &self.home)
            .env("ASTROBIB_LIBRARY", &self.library)
            .env("ASTROBIB_STATE_DIR", &self.state)
            .current_dir(cwd)
            .args(args);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("failed to run astrobib");
        Run {
            cmd: args.join(" "),
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// metrics.json with the given priorities, timestamped now (so
    /// decay does not move them during the test).
    fn seed_metrics(&self, papers: &[(&str, f64)]) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let map: serde_json::Map<String, serde_json::Value> = papers
            .iter()
            .map(|(k, p)| {
                (
                    k.to_string(),
                    serde_json::json!({ "priority": p, "priority_at": now }),
                )
            })
            .collect();
        write(
            self.state.join("metrics.json"),
            &serde_json::to_string(&serde_json::json!({ "version": 1, "papers": map })).unwrap(),
        );
    }

    fn metrics_keys(&self) -> Vec<String> {
        let text = std::fs::read_to_string(self.state.join("metrics.json")).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
        let mut keys: Vec<String> = v["papers"]
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        keys
    }

    fn state_json(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.state.join("state.json")).unwrap_or_default();
        serde_json::from_str(&text).unwrap_or(serde_json::json!({}))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Run {
    cmd: String,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }
    fn code(&self) -> i32 {
        self.code.unwrap_or(-1)
    }
    fn has(&self, needle: &str) -> bool {
        self.stdout.contains(needle) || self.stderr.contains(needle)
    }
    /// Everything a failing assertion needs to be diagnosed.
    fn report(&self) -> String {
        format!(
            "\n$ astrobib {}\nexit {}\n--- stdout ---\n{}--- stderr ---\n{}",
            self.cmd,
            self.code(),
            self.stdout,
            self.stderr
        )
    }
}

// ── helpers ─────────────────────────────────────────────────────────

fn write(path: PathBuf, text: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn fixture(name: &str) -> Data {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap();
    bib::parse_entry(&text).unwrap()
}

/// A fixture entry with fields overridden and its cite key regenerated —
/// a new paper whose key is reproducible from its own data, so the
/// offline fast paths (tidy, import, adopt) accept it without ADS.
fn variant(name: &str, fields: &[(&str, &str)]) -> (String, Data) {
    let mut d = fixture(name);
    for (k, v) in fields {
        d.insert(k.to_string(), v.to_string());
    }
    let key = astrobib::keys::generate_key(&d);
    d.insert("ID".to_string(), key.clone());
    (key, d)
}

fn ads_enabled() -> bool {
    std::env::var("RUN_ADS_TESTS").as_deref() == Ok("1") && std::env::var("ADS_API_TOKEN").is_ok()
}

// ── list / show / search ────────────────────────────────────────────

#[test]
fn list_shows_the_library_newest_first_and_honours_the_limit() {
    let sb = Sandbox::new("list");
    let r = sb.run(&["list"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("5 result(s)"), "{}", r.report());
    // short keys, not full hash keys; newest first
    let first = r.stdout.lines().next().unwrap();
    assert!(first.starts_with("Cabrera2024"), "{}", r.report());
    assert!(!r.stdout.contains("Cabrera2024txuze"), "{}", r.report());

    let r = sb.run(&["list", "-n", "2"]);
    assert!(r.ok(), "{}", r.report());
    assert_eq!(r.stdout.lines().count(), 3, "{}", r.report());
    assert!(r.stdout.contains("2 result(s)"), "{}", r.report());
}

#[test]
fn show_resolves_full_keys_prefixes_and_reports_ambiguity() {
    let sb = Sandbox::new("show");
    // a second Baxter2019 paper makes the base key ambiguous
    let (other, data) = variant(
        "Baxter2019equxm.bib",
        &[
            ("eprint", "1904.11111"),
            ("title", "{A second Baxter paper}"),
            ("adsurl", "https://ui.adsabs.harvard.edu/abs/2019ApJ...900...11B"),
        ],
    );
    write(sb.bib_dir().join(format!("{other}.bib")), &bib::format_entry(&data));

    let r = sb.run(&["show", "Baxter2019equxm"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("@article{Baxter2019equxm,"), "{}", r.report());

    // unambiguous prefix
    let r = sb.run(&["show", "Anders"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("@article{Andersson2021pombz,"), "{}", r.report());

    // bibcode
    let r = sb.run(&["show", "2021ApJ...912...77A"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("@article{Andersson2021pombz,"), "{}", r.report());

    // ambiguous prefix: exit 1 with a did-you-mean list
    let r = sb.run(&["show", "Baxter2019"]);
    assert_eq!(r.code(), 1, "{}", r.report());
    assert!(r.stderr.contains("Ambiguous key 'Baxter2019'"), "{}", r.report());
    assert!(r.stderr.contains("did you mean"), "{}", r.report());
    assert_eq!(r.stderr.lines().count(), 3, "{}", r.report());
    assert!(r.stdout.is_empty(), "{}", r.report());

    // missing
    let r = sb.run(&["show", "Nobody2000zzzzz"]);
    assert_eq!(r.code(), 1, "{}", r.report());
    assert!(r.stderr.contains("Nobody2000zzzzz not found."), "{}", r.report());
}

#[test]
fn search_filters_the_library_locally() {
    let sb = Sandbox::new("search");
    let r = sb.run(&["search", "jet"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("Andersson2021"), "{}", r.report());
    assert!(!r.stdout.contains("Baxter2019"), "{}", r.report());

    // fielded terms and first-author sugar
    let r = sb.run(&["search", "^baxter"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("1 result(s)"), "{}", r.report());
    let r = sb.run(&["search", "year:2018-2019"]);
    assert!(r.stdout.contains("2 result(s)"), "{}", r.report());
    let r = sb.run(&["search", "zzzznothing"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("0 result(s)"), "{}", r.report());
}

#[test]
fn search_metric_tokens_read_the_metrics_store() {
    let sb = Sandbox::new("search-metrics");
    sb.seed_metrics(&[("Andersson2021pombz", 0.9), ("Baxter2019equxm", 0.2)]);

    let r = sb.run(&["search", "pri:>0.5"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("Andersson2021"), "{}", r.report());
    assert!(r.stdout.contains("1 result(s)"), "{}", r.report());

    let r = sb.run(&["search", "pri:>0.1"]);
    assert!(r.stdout.contains("2 result(s)"), "{}", r.report());
    let r = sb.run(&["search", "pri:>0.95"]);
    assert!(r.stdout.contains("0 result(s)"), "{}", r.report());
    // a paper with no metric never matches a metric comparison
    let r = sb.run(&["search", "pri:<0.5"]);
    assert!(r.stdout.contains("1 result(s)"), "{}", r.report());
}

// ── rm ──────────────────────────────────────────────────────────────

#[test]
fn rm_removes_from_every_tier() {
    let sb = Sandbox::new("rm");
    let ms = sb.ms("paper");
    std::fs::copy(
        sb.bib_dir().join("Cabrera2024txuze.bib"),
        ms.join("bib/Cabrera2024txuze.bib"),
    )
    .unwrap();

    let r = sb.run_in(&ms, &["rm", "Cabrera"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.starts_with("Removed Cabrera2024txuze"), "{}", r.report());
    assert!(!sb.bib_dir().join("Cabrera2024txuze.bib").exists());
    assert!(!ms.join("bib/Cabrera2024txuze.bib").exists());

    let r = sb.run_in(&ms, &["rm", "Nobody2000zzzzz"]);
    assert_eq!(r.code(), 1, "{}", r.report());
    assert!(r.stderr.contains("not found"), "{}", r.report());
}

#[test]
fn rm_local_only_rescues_a_sole_copy_into_the_global_tier() {
    let sb = Sandbox::new("rm-local");
    let ms = sb.ms("paper");
    // a manuscript-only entry (a coauthor's drop-in), absent globally
    let (key, data) = variant(
        "Delacroix2018jdgxd.bib",
        &[
            ("eprint", "1802.22222"),
            ("title", "{A coauthor contribution}"),
            ("adsurl", "https://ui.adsabs.harvard.edu/abs/2018ApJ...800...22D"),
        ],
    );
    write(ms.join(format!("bib/{key}.bib")), &bib::format_entry(&data));

    let r = sb.run_in(&ms, &["rm", &key, "--local-only"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("sole copy rescued into the global library"), "{}", r.report());
    assert!(!ms.join(format!("bib/{key}.bib")).exists());
    let rescued = sb.bib_dir().join(format!("{key}.bib"));
    assert!(rescued.exists(), "sole copy was destroyed, not rescued");
    assert!(read(rescued).contains("A coauthor contribution"));

    // a copy that exists in both tiers leaves the global one alone
    std::fs::copy(
        sb.bib_dir().join("Ekwueme2023ophaj.bib"),
        ms.join("bib/Ekwueme2023ophaj.bib"),
    )
    .unwrap();
    let r = sb.run_in(&ms, &["rm", "Ekwueme", "--local-only"]);
    assert!(r.ok(), "{}", r.report());
    assert!(!r.stdout.contains("rescued"), "{}", r.report());
    assert!(sb.bib_dir().join("Ekwueme2023ophaj.bib").exists());

    // --local-only without a local tier is an error
    let r = sb.run(&["rm", "Ekwueme", "--local-only"]);
    assert_eq!(r.code(), 1, "{}", r.report());
    assert!(r.stderr.contains("needs a local"), "{}", r.report());
}

// ── refs ────────────────────────────────────────────────────────────

#[test]
fn refs_syncs_a_tex_manuscript_and_keys_by_the_cited_string() {
    let sb = Sandbox::new("refs-tex");
    let ms = sb.ms("paper");
    write(
        ms.join("main.tex"),
        "\\documentclass{article}\n\\begin{document}\n\
         Jets \\citep{Andersson2021} and \\citet{Baxter2019equxm}.\n\
         \\end{document}\n",
    );
    // an uncited member of the manuscript db
    std::fs::copy(
        sb.bib_dir().join("Cabrera2024txuze.bib"),
        ms.join("bib/Cabrera2024txuze.bib"),
    )
    .unwrap();

    // --dry-run touches nothing
    let r = sb.run_in(&ms, &["refs", "--dry-run"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("would write"), "{}", r.report());
    assert!(!ms.join("refs.bib").exists(), "dry run wrote refs.bib");
    assert!(!ms.join("bib/Andersson2021pombz.bib").exists(), "dry run copied entries");

    let r = sb.run_in(&ms, &["refs"]);
    assert!(r.ok(), "{}", r.report());
    assert!(ms.join("bib/Andersson2021pombz.bib").exists(), "{}", r.report());
    assert!(ms.join("bib/Baxter2019equxm.bib").exists(), "{}", r.report());
    let refs = read(ms.join("refs.bib"));
    // keyed by the string actually cited, not the full hash key
    assert!(refs.contains("@article{Andersson2021,"), "refs.bib:\n{refs}");
    assert!(!refs.contains("@article{Andersson2021pombz,"), "refs.bib:\n{refs}");
    assert!(refs.contains("@article{Baxter2019equxm,"), "refs.bib:\n{refs}");
    // uncited members never reach refs.bib
    assert!(!refs.contains("Cabrera"), "refs.bib:\n{refs}");
    assert!(r.stdout.contains("2 entries"), "{}", r.report());
    assert!(r.stdout.contains("1 uncited entry"), "{}", r.report());

    // idempotent from the second run on: the first regeneration after
    // entries are copied in re-serializes them from disk, which flips
    // the trailing non-FIELD_ORDER fields (the documented two-form
    // oscillation in CLAUDE.md) — both forms are canonical
    let second = sb.run_in(&ms, &["refs"]);
    assert!(second.ok(), "{}", second.report());
    let settled = read(ms.join("refs.bib"));
    let again = sb.run_in(&ms, &["refs"]);
    assert!(again.stdout.contains("(unchanged)"), "{}", again.report());
    assert_eq!(settled, read(ms.join("refs.bib")));

    // --prune drops the uncited member (it lives globally too, so no rescue)
    let r = sb.run_in(&ms, &["refs", "--prune"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("Pruned 1 uncited entry"), "{}", r.report());
    assert!(!ms.join("bib/Cabrera2024txuze.bib").exists());
    assert!(sb.bib_dir().join("Cabrera2024txuze.bib").exists(), "prune touched the global tier");

    // a cite that resolves nowhere is reported, not invented
    write(ms.join("main.tex"), "\\citep{Andersson2021, Nobody2000zzzzz}\n");
    let r = sb.run_in(&ms, &["refs"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stderr.contains("missing: Nobody2000zzzzz"), "{}", r.report());
}

#[test]
fn refs_renders_a_markdown_bibliography_between_markers() {
    let sb = Sandbox::new("refs-md");
    let ms = sb.ms("notes");
    let prose = "# Review\n\nJets are discussed by @Andersson2021 and mail me at jane@example.com.\n\nA closing paragraph.\n";
    write(ms.join("main.md"), prose);

    let r = sb.run_in(&ms, &["refs"]);
    assert!(r.ok(), "{}", r.report());
    let md = read(ms.join("main.md"));
    assert!(md.starts_with(prose.trim_end_matches('\n')), "prose was rewritten:\n{md}");
    assert!(md.contains("## References"), "{md}");
    assert!(md.contains("<!-- astrobib:references -->"), "{md}");
    assert!(md.contains("<!-- /astrobib:references -->"), "{md}");
    assert!(md.contains("Andersson, F."), "{md}");
    // one bibliography item: an email address is not a citation
    assert!(r.stdout.contains("1 reference(s)"), "{}", r.report());
    let block = md.split("<!-- astrobib:references -->").nth(1).unwrap();
    assert_eq!(block.lines().filter(|l| l.starts_with("- ")).count(), 1, "{md}");
    assert!(md.contains("A closing paragraph."), "{md}");
    // a pure-markdown manuscript gets no refs.bib unless asked
    assert!(!ms.join("refs.bib").exists(), "{}", r.report());

    // idempotent: same block, file untouched
    let again = sb.run_in(&ms, &["refs"]);
    assert!(again.stdout.contains("(unchanged)"), "{}", again.report());
    assert_eq!(md, read(ms.join("main.md")));

    // regenerated wholesale, and only between the markers
    write(ms.join("main.md"), &md.replace("@Andersson2021", "@Baxter2019equxm"));
    let r = sb.run_in(&ms, &["refs"]);
    assert!(r.ok(), "{}", r.report());
    let md2 = read(ms.join("main.md"));
    assert!(md2.contains("Baxter, M."), "{md2}");
    assert!(!md2.contains("Andersson, F."), "{md2}");
    assert_eq!(md2.matches("## References").count(), 1, "{md2}");
    assert!(md2.contains("A closing paragraph."), "{md2}");
}

// ── convert ─────────────────────────────────────────────────────────

#[test]
fn convert_rewrites_cite_braces_only() {
    let sb = Sandbox::new("convert");
    let ms = sb.ms("paper");
    let body = "Andersson2021pombz is prose, \\citep[e.g.][]{Andersson2021pombz} is a cite.\n";
    write(ms.join("main.tex"), body);

    // --dry-run reports and writes nothing
    let r = sb.run_in(&ms, &["convert", "short", "--dry-run"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("Andersson2021pombz → Andersson2021"), "{}", r.report());
    assert!(r.stdout.contains("would rewrite 1 cite key(s)"), "{}", r.report());
    assert_eq!(read(ms.join("main.tex")), body, "dry run rewrote the source");
    assert!(!ms.join("refs.bib").exists(), "dry run wrote refs.bib");

    let r = sb.run_in(&ms, &["convert", "short"]);
    assert!(r.ok(), "{}", r.report());
    let tex = read(ms.join("main.tex"));
    assert!(tex.contains("\\citep[e.g.][]{Andersson2021}"), "{tex}");
    // prose mentioning the same string is never touched
    assert!(tex.starts_with("Andersson2021pombz is prose"), "{tex}");
    // refs.bib regenerated to match the new cite key
    let refs = read(ms.join("refs.bib"));
    assert!(refs.contains("@article{Andersson2021,"), "{refs}");

    // bibcode form, and back to full
    let r = sb.run_in(&ms, &["convert", "bibcode"]);
    assert!(r.ok(), "{}", r.report());
    let tex = read(ms.join("main.tex"));
    assert!(tex.contains("{2021ApJ...912...77A}"), "{tex}");
    assert!(tex.starts_with("Andersson2021pombz is prose"), "{tex}");
    assert!(read(ms.join("refs.bib")).contains("@article{2021ApJ...912...77A,"));

    let r = sb.run_in(&ms, &["convert", "full"]);
    assert!(r.ok(), "{}", r.report());
    assert!(read(ms.join("main.tex")).contains("{Andersson2021pombz}"));
    let r = sb.run_in(&ms, &["convert", "full"]);
    assert!(r.stdout.contains("already in full form"), "{}", r.report());
}

#[test]
fn convert_without_a_manuscript_fails_cleanly() {
    let sb = Sandbox::new("convert-nowhere");
    let r = sb.run(&["convert", "short"]);
    assert_eq!(r.code(), 1, "{}", r.report());
    assert!(r.stderr.contains("needs a manuscript directory"), "{}", r.report());

    // a bib root with no .tex sources is equally refused
    let ms = sb.ms("empty-paper");
    let r = sb.run_in(&ms, &["convert", "short"]);
    assert_eq!(r.code(), 1, "{}", r.report());
    assert!(r.stderr.contains("No .tex sources found"), "{}", r.report());
}

// ── tidy ────────────────────────────────────────────────────────────

#[test]
fn tidy_leaves_a_canonical_bib_dir_alone() {
    let sb = Sandbox::new("tidy-canonical");
    let ms = sb.ms("paper");
    write(ms.join("main.tex"), "\\citep{Andersson2021pombz}\n");
    let src = sb.bib_dir().join("Andersson2021pombz.bib");
    std::fs::copy(&src, ms.join("bib/Andersson2021pombz.bib")).unwrap();
    let before = read(ms.join("bib/Andersson2021pombz.bib"));

    let r = sb.run_in(&ms, &["tidy"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("bib/ is already canonical."), "{}", r.report());
    assert_eq!(before, read(ms.join("bib/Andersson2021pombz.bib")));
    assert_eq!(read(ms.join("main.tex")), "\\citep{Andersson2021pombz}\n");
    assert!(read(ms.join("refs.bib")).contains("@article{Andersson2021pombz,"));
}

/// tidy owns tags/ too: sorted, deduped, comments kept — and a key it
/// cannot resolve stays, because the whole point of the format is that
/// a line naming a paper you have not imported yet is legitimate.
#[test]
fn tidy_canonicalizes_tag_files_without_dropping_unknown_keys() {
    let sb = Sandbox::new("tidy-tags");
    let ms = sb.ms("paper");
    write(ms.join("main.tex"), "\\citep{Andersson2021pombz}\n");
    std::fs::copy(
        sb.bib_dir().join("Andersson2021pombz.bib"),
        ms.join("bib/Andersson2021pombz.bib"),
    )
    .unwrap();
    std::fs::create_dir_all(ms.join("tags")).unwrap();
    write(
        ms.join("tags/section-3"),
        "# the spiral-shock references\nZrake2019notyet\nAndersson2021pombz\nZrake2019notyet\n",
    );

    let r = sb.run_in(&ms, &["tidy"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("Tidied tags/: section-3"), "{}", r.report());
    assert_eq!(
        read(ms.join("tags/section-3")),
        "# the spiral-shock references\nAndersson2021pombz\nZrake2019notyet\n",
    );

    // idempotent: a second pass has nothing to say
    let r = sb.run_in(&ms, &["tidy"]);
    assert!(r.ok(), "{}", r.report());
    assert!(!r.stdout.contains("Tidied tags/"), "{}", r.report());
}

#[test]
fn tidy_canonicalizes_a_foreign_file_without_ads_when_the_key_is_reproducible() {
    let sb = Sandbox::new("tidy-foreign");
    let ms = sb.ms("paper");
    // a coauthor's drop-in: right data, wrong filename, entry already
    // keyed canonically (the reproducible-key fast path, so no ADS)
    let (key, data) = variant(
        "Ekwueme2023ophaj.bib",
        &[
            ("eprint", "2303.33333"),
            ("title", "{A dropped-in coauthor reference}"),
            ("adsurl", "https://ui.adsabs.harvard.edu/abs/2023ApJ...700...33E"),
        ],
    );
    write(ms.join("bib/from-coauthor.bib"), &bib::format_entry(&data));
    write(ms.join("main.tex"), &format!("\\citep{{{key}}}\n"));

    let r = sb.run_in(&ms, &["tidy"]);
    assert!(r.ok(), "{}", r.report());
    assert!(ms.join(format!("bib/{key}.bib")).exists(), "{}", r.report());
    assert!(!ms.join("bib/from-coauthor.bib").exists(), "{}", r.report());
    assert!(r.stdout.contains("1 entry canonicalized"), "{}", r.report());
    // the entry is bibdata, so it lands in the personal library too
    assert!(sb.bib_dir().join(format!("{key}.bib")).exists());
    let refs = read(ms.join("refs.bib"));
    assert!(refs.contains(&format!("@article{{{key},")), "{refs}");
}

#[test]
fn tidy_adopts_a_legacy_manuscript_offline() {
    let sb = Sandbox::new("adopt");
    let dir = sb.dir("legacy");
    // (a) a paper the library has never seen, already canonically keyed
    let (fresh, fresh_data) = variant(
        "Delacroix2018jdgxd.bib",
        &[
            ("eprint", "1806.44444"),
            ("title", "{A legacy manuscript reference}"),
            ("adsurl", "https://ui.adsabs.harvard.edu/abs/2018ApJ...860...44D"),
        ],
    );
    // (b) the same paper the library holds under a different key —
    // adoption keeps the existing key and rewrites the cite
    let (dupe, dupe_data) = variant("Cabrera2024txuze.bib", &[("eprint", "2402.55555")]);
    write(
        dir.join("oldrefs.bib"),
        &format!("{}\n{}", bib::format_entry(&fresh_data), bib::format_entry(&dupe_data)),
    );
    write(
        dir.join("main.tex"),
        &format!("Intro \\citep{{{fresh}}} and \\citet{{{dupe}}}.\n"),
    );

    let r = sb.run_in(&dir, &["tidy"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("Adopting"), "{}", r.report());
    // bib/ created and populated
    assert!(dir.join(format!("bib/{fresh}.bib")).exists(), "{}", r.report());
    // the duplicate resolved to the key already in the library
    assert!(!dir.join(format!("bib/{dupe}.bib")).exists(), "{}", r.report());
    // cites rewritten only where the key changed
    let tex = read(dir.join("main.tex"));
    assert!(tex.contains(&format!("\\citep{{{fresh}}}")), "{tex}");
    assert!(tex.contains("\\citet{Cabrera2024}"), "{tex}");
    // refs.bib regenerated canonically, the loose file left in place
    let refs = read(dir.join("refs.bib"));
    assert!(refs.contains(&format!("@article{{{fresh},")), "{refs}");
    assert!(refs.contains("@article{Cabrera2024,"), "{refs}");
    assert!(dir.join("oldrefs.bib").exists());
    assert!(r.stdout.contains("2 entries adopted"), "{}", r.report());
}

#[test]
fn adoption_that_needs_ads_fails_without_destroying_anything() {
    let sb = Sandbox::new("adopt-offline");
    let dir = sb.dir("legacy");
    // a foreign key that is NOT reproducible from its own data: the only
    // way to canonicalize it is an ADS lookup, which is unavailable here
    let mut data = fixture("Baxter2019equxm.bib");
    data.insert("ID".to_string(), "baxter_frb_2019".to_string());
    let loose = format!("{}\n", bib::format_entry(&data));
    write(dir.join("refs.bib"), &loose);
    let tex = "Intro \\citep{baxter_frb_2019}.\n";
    write(dir.join("main.tex"), tex);

    let r = sb.run_in(&dir, &["tidy"]);
    assert_ne!(r.code(), 0, "a no-op adoption must not report success{}", r.report());
    assert!(r.has("left alone"), "{}", r.report());
    // nothing was rewritten, nothing was truncated
    assert_eq!(read(dir.join("refs.bib")), loose, "the user's refs.bib was rewritten");
    assert_eq!(read(dir.join("main.tex")), tex, "the manuscript source was rewritten");
    assert!(!dir.join("bib").exists(), "an empty bib/ was left behind");
}

#[test]
fn refs_never_truncates_an_existing_bibliography_to_nothing() {
    let sb = Sandbox::new("refs-guard");
    let ms = sb.ms("paper");
    // a hand-maintained refs.bib whose keys astrobib cannot resolve
    let mut data = fixture("Baxter2019equxm.bib");
    data.insert("ID".to_string(), "baxter_frb_2019".to_string());
    let loose = format!("{}\n", bib::format_entry(&data));
    write(ms.join("refs.bib"), &loose);
    write(ms.join("main.tex"), "\\citep{baxter_frb_2019}\n");

    let r = sb.run_in(&ms, &["refs"]);
    assert_eq!(read(ms.join("refs.bib")), loose, "refs.bib was emptied");
    assert!(r.has("refusing"), "{}", r.report());
}

#[test]
fn tidy_resolves_a_foreign_key_against_ads() {
    if !ads_enabled() {
        eprintln!("skipped: set RUN_ADS_TESTS=1 and ADS_API_TOKEN to run ADS tests");
        return;
    }
    let sb = Sandbox::new("tidy-ads");
    let ms = sb.ms("paper");
    let mut data = fixture("Baxter2019equxm.bib");
    // a real arXiv ID so ADS can resolve it; the key is foreign
    data.insert("eprint".to_string(), "1710.05931".to_string());
    data.shift_remove("doi");
    data.shift_remove("adsurl");
    data.insert("ID".to_string(), "foreign_key_2017".to_string());
    write(ms.join("bib/coauthor.bib"), &bib::format_entry(&data));
    write(ms.join("main.tex"), "\\citep{foreign_key_2017}\n");

    let token = std::env::var("ADS_API_TOKEN").unwrap();
    let r = sb.run_env(&ms, &["tidy"], &[("ADS_API_TOKEN", &token)]);
    assert!(r.ok(), "{}", r.report());
    assert!(!ms.join("bib/coauthor.bib").exists(), "{}", r.report());
    assert!(!read(ms.join("main.tex")).contains("foreign_key_2017"), "{}", r.report());
}

// ── config ──────────────────────────────────────────────────────────

#[test]
fn config_reports_the_resolved_environment_and_saves_fields() {
    let sb = Sandbox::new("config");
    let r = sb.run(&["config"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains(&format!("astrobib {}", env!("CARGO_PKG_VERSION"))), "{}", r.report());
    assert!(
        r.stdout.contains(&format!("{}  ($ASTROBIB_LIBRARY, 5 entries)", sb.library.display())),
        "{}",
        r.report()
    );
    assert!(r.stdout.contains("local library    none"), "{}", r.report());
    assert!(r.stdout.contains("ADS token        MISSING"), "{}", r.report());
    assert!(r.stdout.contains("email            not set"), "{}", r.report());

    let r = sb.run(&["config", "email", "jane@example.edu"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("email saved."), "{}", r.report());
    assert_eq!(sb.state_json()["email"], "jane@example.edu");

    let r = sb.run(&["config"]);
    assert!(r.stdout.contains("email            jane@example.edu"), "{}", r.report());

    // a local tier is reported when one is in scope
    let ms = sb.ms("paper");
    std::fs::copy(
        sb.bib_dir().join("Cabrera2024txuze.bib"),
        ms.join("bib/Cabrera2024txuze.bib"),
    )
    .unwrap();
    let r = sb.run_in(&ms, &["config"]);
    assert!(
        r.stdout.contains(&format!("local library    {}  (1 entries)", ms.display())),
        "{}",
        r.report()
    );
}

// ── gc ──────────────────────────────────────────────────────────────

/// A sandbox with something in every cache: two PDFs, a query cache at
/// its cache-dir home, and two curated metrics.
fn gc_sandbox(tag: &str) -> Sandbox {
    let sb = Sandbox::new(tag);
    write(sb.pdf_cache().join("Andersson2021pombz.pdf"), "%PDF-1.4 one\n");
    write(sb.pdf_cache().join("Baxter2019equxm.pdf"), "%PDF-1.4 two\n");
    write(
        sb.query_cache(),
        r#"{"version": 1, "tabs": {"tt1": [], "tt2": []}}"#,
    );
    sb.seed_metrics(&[("Andersson2021pombz", 0.5), ("Ghost2001aaaaa", 0.9)]);
    sb
}

#[test]
fn gc_reports_what_the_caches_cost_and_deletes_nothing() {
    let sb = gc_sandbox("gc");
    let r = sb.run(&["gc"]);
    assert!(r.ok(), "{}", r.report());
    assert!(
        r.stdout.contains(&format!("PDF cache        {}  (2 file(s)", sb.pdf_cache().display())),
        "{}",
        r.report()
    );
    assert!(
        r.stdout.contains(&format!("query cache      {}", sb.query_cache().display())),
        "{}",
        r.report()
    );
    assert!(
        r.stdout.contains(&format!("metrics          {}  (2 paper(s)", sb.state.join("metrics.json").display())),
        "{}",
        r.report()
    );
    // the closing advice: the cache dir is the user's to delete
    assert!(
        r.stdout.contains(&format!("rm -rf {}", sb.home.join(".cache/astrobib").display())),
        "{}",
        r.report()
    );
    assert!(r.stdout.contains("never deletes it for you"), "{}", r.report());
    assert!(r.stdout.contains("metrics.json is not cache"), "{}", r.report());

    // a report reports: every byte is still there afterwards
    assert!(sb.pdf_cache().join("Andersson2021pombz.pdf").exists());
    assert!(sb.pdf_cache().join("Baxter2019equxm.pdf").exists());
    assert!(sb.query_cache().exists());
    assert_eq!(sb.metrics_keys(), ["Andersson2021pombz", "Ghost2001aaaaa"]);
}

#[test]
fn gc_has_no_deleting_flags_at_all() {
    let sb = gc_sandbox("gc-flags");
    for flag in ["--clean", "--metrics", "--prune"] {
        let r = sb.run(&["gc", flag]);
        assert_eq!(r.code(), 2, "{}", r.report());
        assert!(sb.pdf_cache().join("Baxter2019equxm.pdf").exists());
    }
}

#[test]
fn gc_on_a_machine_with_no_caches_reports_zeroes() {
    let sb = Sandbox::empty("gc-cold");
    let r = sb.run(&["gc"]);
    assert!(r.ok(), "{}", r.report());
    assert!(r.stdout.contains("(0 file(s), 0 KB)"), "{}", r.report());
    assert!(r.stdout.contains("(0 paper(s), 0 KB)"), "{}", r.report());
}

#[test]
fn the_query_cache_lives_in_the_cache_dir_not_the_state_dir() {
    let sb = Sandbox::new("query-cache");
    // this test exercises the library in-process, so it points $HOME at
    // the sandbox; every other test passes the environment explicitly
    // to a child, so nothing else can see this
    std::env::set_var("HOME", &sb.home);
    std::env::set_var("ASTROBIB_STATE_DIR", &sb.state);

    assert_eq!(astrobib::tabs::cache_file(), sb.query_cache());
    astrobib::tabs::save_cached_articles("tt1", &[]);
    assert!(sb.query_cache().exists(), "the query cache was not written to the cache dir");
    // curated state is not in the blast radius of rm -rf ~/.cache
    assert!(!sb.state.join("query_cache.json").exists());
    assert_eq!(astrobib::tabs::load_cached_articles("tt1").len(), 0);

    // and gc points at the file that is actually being used
    let r = sb.run(&["gc"]);
    assert!(
        r.stdout.contains(&format!("query cache      {}", sb.query_cache().display())),
        "{}",
        r.report()
    );
}

// ── argument handling ───────────────────────────────────────────────

#[test]
fn a_bogus_positional_argument_exits_two() {
    let sb = Sandbox::new("bogus");
    let r = sb.run(&["lst"]);
    assert_eq!(r.code(), 2, "{}", r.report());
    assert!(r.stderr.contains("neither an astrobib command nor a directory"), "{}", r.report());
    assert!(r.stderr.contains("--help"), "{}", r.report());

    // an explicit directory is a library root, not an error
    let ms = sb.ms("paper");
    let r = sb.run_in(&sb.home, &["list"]);
    assert!(r.ok(), "{}", r.report());
    let r = sb.run(&[ms.to_str().unwrap(), "list"]);
    assert!(r.ok(), "{}", r.report());
}
// ── broken pipe ─────────────────────────────────────────────────────

#[test]
fn output_into_a_closed_pipe_does_not_panic() {
    let sb = Sandbox::new("pipe");
    // enough output to overflow any pipe buffer, so the reader really
    // does close on us mid-stream
    for i in 0..1500 {
        let key = format!("Filler{:04}aaaaa", i);
        write(
            sb.bib_dir().join(format!("{key}.bib")),
            &format!(
                "@article{{{key},\n  author = {{{{Filler}}, A.}},\n  \
                 title = {{Padding entry {i} with a title long enough to fill a pipe buffer}},\n  \
                 year = {{2000}},\n}}\n"
            ),
        );
    }
    let script = format!("{BIN} list -n 100000 | head -2");
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg(&script)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()))
        .env("HOME", &sb.home)
        .env("ASTROBIB_LIBRARY", &sb.library)
        .env("ASTROBIB_STATE_DIR", &sb.state)
        .current_dir(&sb.home)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "panic on broken pipe:\n{stderr}");
    assert!(!stderr.contains("Broken pipe"), "broken-pipe noise on stderr:\n{stderr}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 2);
}

/// The build-facing modes: `--no-sync` produces the bibliography
/// without writing into `bib/` (a make recipe must not modify its own
/// prerequisites) and always advances the mtime (make judges freshness
/// by timestamp, so an unchanged file left with an old mtime makes the
/// rule re-run on every build). `--check` answers the question and
/// writes nothing at all.
#[test]
fn refs_no_sync_generates_without_mutating_bib() {
    let sb = Sandbox::new("refs-nosync");
    let ms = sb.ms("paper");
    write(
        ms.join("main.tex"),
        "\\documentclass{article}\n\\begin{document}\n\
         \\citep{Andersson2021}\n\\end{document}\n",
    );

    let r = sb.run_in(&ms, &["refs", "--no-sync"]);
    assert!(r.ok(), "{}", r.report());
    // the bibliography exists…
    assert!(ms.join("refs.bib").exists(), "{}", r.report());
    // …but the cited entry was NOT copied into the manuscript db
    assert!(
        !ms.join("bib/Andersson2021pombz.bib").exists(),
        "--no-sync mutated bib/: {}",
        r.report()
    );
    // and the run says what it deliberately skipped
    assert!(r.stdout.contains("not in bib/"), "{}", r.report());
}

#[test]
fn refs_stamps_the_mtime_even_when_the_content_is_unchanged() {
    let sb = Sandbox::new("refs-stamp");
    let ms = sb.ms("paper");
    write(
        ms.join("main.tex"),
        "\\documentclass{article}\n\\begin{document}\n\
         \\citep{Andersson2021}\n\\end{document}\n",
    );
    assert!(sb.run_in(&ms, &["refs"]).ok());
    let first = std::fs::metadata(ms.join("refs.bib")).unwrap().modified().unwrap();
    let before = read(ms.join("refs.bib"));

    std::thread::sleep(std::time::Duration::from_millis(1100));
    let r = sb.run_in(&ms, &["refs", "--no-sync"]);
    assert!(r.ok(), "{}", r.report());

    let second = std::fs::metadata(ms.join("refs.bib")).unwrap().modified().unwrap();
    assert_eq!(before, read(ms.join("refs.bib")), "content should not change");
    assert!(
        second > first,
        "mtime must advance so make converges (was {first:?}, now {second:?}): {}",
        r.report()
    );
}

#[test]
fn refs_check_reports_without_writing() {
    let sb = Sandbox::new("refs-check");
    let ms = sb.ms("paper");
    write(
        ms.join("main.tex"),
        "\\documentclass{article}\n\\begin{document}\n\
         \\citep{Andersson2021}\n\\end{document}\n",
    );

    // missing → stale, and still nothing written
    let r = sb.run_in(&ms, &["refs", "--check"]);
    assert!(!r.ok(), "check should fail when refs.bib is absent: {}", r.report());
    assert!(!ms.join("refs.bib").exists(), "check wrote refs.bib");

    assert!(sb.run_in(&ms, &["refs"]).ok());
    let r = sb.run_in(&ms, &["refs", "--check"]);
    assert!(r.ok(), "check should pass on a current file: {}", r.report());
    assert!(r.stdout.contains("current"), "{}", r.report());

    // tampered → stale, and check leaves the tampering in place
    let tampered = read(ms.join("refs.bib")) + "\n% edited by hand\n";
    write(ms.join("refs.bib"), &tampered);
    let r = sb.run_in(&ms, &["refs", "--check"]);
    assert!(!r.ok(), "check should fail on a stale file: {}", r.report());
    assert_eq!(read(ms.join("refs.bib")), tampered, "check modified the file");
}
