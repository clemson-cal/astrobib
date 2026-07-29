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
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/astrobib/pdfs")
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

/// Open a URL in the system browser.
pub fn browser_open(url: &str) {
    // never hand a non-URL to open(1) — it would resolve it as a file
    // path — and silence the child: anything it prints while the TUI
    // owns the terminal in raw mode corrupts the display
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    use std::process::Stdio;
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(not(target_os = "macos"))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn downloads_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Downloads")
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
                    use std::os::unix::fs::MetadataExt;
                    out.insert(p, (st.len(), st.mtime_nsec() + st.mtime() * 1_000_000_000));
                }
            }
        }
    }
    out
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
    use std::process::Stdio;
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        cmd.args(paths).stdout(Stdio::null()).stderr(Stdio::null());
        let _ = cmd.spawn();
    }
    #[cfg(not(target_os = "macos"))]
    for p in paths {
        let _ = std::process::Command::new("xdg-open")
            .arg(p)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}
