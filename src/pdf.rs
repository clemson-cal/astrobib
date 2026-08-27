//! Ephemeral PDF cache: download, browser-watch flow, and user-file
//! import into ~/.cache/astrobib/pdfs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Auto,
    Arxiv,
    Oa,
}

pub fn cache_dir() -> PathBuf {
    crate::library::pdf_cache_dir()
}

pub fn cache_path(key: &str) -> PathBuf {
    cache_dir().join(format!("{key}.pdf"))
}

pub fn is_cached(key: &str) -> bool {
    cache_path(key).exists()
}

/// GET a URL into the cache slot; rejects non-PDF payloads (the
/// content-type must mention pdf or octet-stream).
fn download_url(path: &PathBuf, url: &str) -> Option<PathBuf> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build();
    let resp = agent.get(url).call().ok()?;
    let ctype = resp.header("content-type").unwrap_or("");
    if !ctype.contains("pdf") && !ctype.contains("octet-stream") {
        return None;
    }
    let mut bytes: Vec<u8> = vec![];
    use std::io::Read;
    resp.into_reader().read_to_end(&mut bytes).ok()?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(path, &bytes).ok()?;
    Some(path.clone())
}

pub fn bibcode_from_adsurl(adsurl: &str) -> Option<&str> {
    if adsurl.is_empty() {
        return None;
    }
    let t = adsurl.trim_end_matches('/');
    t.rsplit('/').next().filter(|s| !s.is_empty())
}

/// Return the cached PDF path, downloading if needed.
/// Auto: ADS OA_PDF resolver first, then arXiv fallback.
pub fn fetch(key: &str, eprint: &str, adsurl: &str) -> Option<PathBuf> {
    let path = cache_path(key);
    if path.exists() {
        return Some(path);
    }
    fetch_source(key, eprint, adsurl, Source::Auto)
}

/// Source-specific fetch for the pub card's per-source buttons; always
/// forces a re-download, replacing any cached copy.
pub fn fetch_source(key: &str, eprint: &str, adsurl: &str, source: Source) -> Option<PathBuf> {
    let path = cache_path(key);
    if path.exists() {
        std::fs::remove_file(&path).ok()?;
    }
    let try_oa = || -> Option<PathBuf> {
        let bc = bibcode_from_adsurl(adsurl)?;
        // OA copy first; older papers live on ADS's scan service
        let url = crate::ads::resolve_pdf_url(bc, "OA_PDF")
            .or_else(|| crate::ads::resolve_pdf_url(bc, "ADS_PDF"))?;
        download_url(&path, &url)
    };
    let try_arxiv = || -> Option<PathBuf> {
        if eprint.is_empty() {
            return None;
        }
        download_url(&path, &format!("https://arxiv.org/pdf/{}", eprint.trim()))
    };
    match source {
        Source::Arxiv => try_arxiv(),
        Source::Oa => try_oa(),
        Source::Auto => try_oa().or_else(try_arxiv),
    }
}

/// Best URL for manual PDF download, verified against the ADS resolver.
/// Makes a network call; run off the UI thread.
pub fn browser_resolve_url(doi: &str, adsurl: &str, eprint: &str) -> Option<String> {
    if let Some(bc) = bibcode_from_adsurl(adsurl) {
        if let Some(url) = crate::ads::resolve_pdf_url(bc, "PUB_PDF") {
            return Some(url);
        }
        // older papers: ADS's own scanned article service
        if let Some(url) = crate::ads::resolve_pdf_url(bc, "ADS_PDF") {
            return Some(url);
        }
    }
    if !doi.is_empty() {
        return Some(format!("https://doi.org/{doi}"));
    }
    if !eprint.is_empty() {
        return Some(format!("https://arxiv.org/abs/{}", eprint.trim()));
    }
    None
}

/// Hand arguments to the platform's default-application launcher. The
/// child is silenced in every arm: anything it prints while the TUI owns
/// the terminal in raw mode corrupts the display.
///
/// macOS `open` takes the whole list at once; `xdg-open` and Windows
/// take one target per invocation. No arm goes through a shell, so a
/// `&` in an ADS query URL — or a space in a filename — is passed
/// through literally rather than being re-parsed as syntax.
fn shell_open<'a>(targets: impl Iterator<Item = &'a std::ffi::OsStr>) {
    use std::process::{Command, Stdio};
    let spawn = |mut c: Command| {
        let _ = c.stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    };
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        let mut any = false;
        for t in targets {
            c.arg(t);
            any = true;
        }
        if any {
            spawn(c);
        }
    }
    #[cfg(windows)]
    for t in targets {
        // explorer.exe is the launcher reachable without linking
        // ShellExecuteW, and unlike `cmd /c start` it parses its command
        // line by the ordinary CRT rules Command already quotes for. It
        // reports a nonzero exit status even on success, which is why
        // nothing here inspects one.
        let mut c = Command::new("explorer.exe");
        c.arg(t);
        spawn(c);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    for t in targets {
        let mut c = Command::new("xdg-open");
        c.arg(t);
        spawn(c);
    }
}

/// Open a URL in the system browser.
pub fn browser_open(url: &str) {
    // never hand a non-URL to open(1) — it would resolve it as a file
    // path — and silence the child: anything it prints while the TUI
    // owns the terminal in raw mode corrupts the display
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    shell_open(std::iter::once(std::ffi::OsStr::new(url)));
}

fn downloads_dir() -> PathBuf {
    crate::library::home_dir().join("Downloads")
}

/// PDFs currently in ~/Downloads: path → (size, mtime_ns). Size+mtime
/// lets the poller catch a download overwriting a same-named file.
pub fn downloads_snapshot() -> HashMap<PathBuf, (u64, i64)> {
    let mut out = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(downloads_dir()) {
        for f in rd.flatten() {
            let p = f.path();
            let is_pdf = p
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("pdf"));
            if is_pdf && p.is_file() {
                if let Ok(st) = p.metadata() {
                    out.insert(p, (st.len(), mtime_nanos(&st)));
                }
            }
        }
    }
    out
}

/// Modification time in nanoseconds since the Unix epoch. `SystemTime`
/// is the portable spelling of what `MetadataExt::mtime` gives on Unix —
/// Windows has no `st_mtime`, and its file times are FILETIME-derived.
/// An unreadable or pre-epoch timestamp collapses to 0; the poller only
/// ever compares this against the value it recorded for the same path,
/// so such a file simply never looks like it changed.
fn mtime_nanos(st: &std::fs::Metadata) -> i64 {
    st.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
}

fn looks_like_pdf(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes
        .windows(4)
        .take(1021)
        .any(|w| w == b"%PDF") // header may follow a short preamble
}

/// Watch ~/Downloads for a new or rewritten PDF; move it into the cache
/// on arrival. A file must hold the same size across two polls before it
/// is taken, so a partial download is never grabbed.
pub fn poll_downloads(
    key: &str,
    before: &HashMap<PathBuf, (u64, i64)>,
    timeout_secs: u64,
    cancel: &AtomicBool,
) -> Option<PathBuf> {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut prev_sizes: HashMap<PathBuf, u64> = HashMap::new();
    while std::time::Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        std::thread::sleep(Duration::from_secs(1));
        for (f, (size, mtime)) in downloads_snapshot() {
            if before.get(&f) == Some(&(size, mtime)) {
                continue; // pre-existing file, unchanged
            }
            if prev_sizes.get(&f) == Some(&size) && size > 0 {
                if !looks_like_pdf(&f) {
                    continue;
                }
                let dest = cache_path(key);
                std::fs::create_dir_all(dest.parent()?).ok()?;
                std::fs::rename(&f, &dest)
                    .or_else(|_| std::fs::copy(&f, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&f)))
                    .ok()?;
                return Some(dest);
            }
            prev_sizes.insert(f, size);
        }
    }
    None
}

/// Copy a user-chosen PDF into the cache. Copies rather than moves;
/// rejects files without a %PDF header.
pub fn import_file(key: &str, source: &Path) -> Option<PathBuf> {
    if !looks_like_pdf(source) {
        return None;
    }
    let dest = cache_path(key);
    std::fs::create_dir_all(dest.parent()?).ok()?;
    std::fs::copy(source, &dest).ok()?;
    Some(dest)
}

/// PDFs in ~/Downloads, newest first — the pick-file candidate list.
pub fn downloads_pdfs() -> Vec<PathBuf> {
    let mut files: Vec<(PathBuf, i64)> = downloads_snapshot()
        .into_iter()
        .map(|(p, (_, m))| (p, m))
        .collect();
    files.sort_by_key(|(_, m)| -*m);
    files.into_iter().map(|(p, _)| p).collect()
}

/// Open cached PDFs with the platform opener.
pub fn open_paths(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    shell_open(paths.iter().map(|p| p.as_os_str()));
}

