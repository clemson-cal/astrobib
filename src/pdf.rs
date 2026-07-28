//! Ephemeral PDF cache — port of the download side of astrobib/pdf.py
//! (browser-watch flow comes later).

use std::path::PathBuf;
use std::time::Duration;

pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/astrobib/pdfs")
}

pub fn cache_path(key: &str) -> PathBuf {
    cache_dir().join(format!("{key}.pdf"))
}

pub fn is_cached(key: &str) -> bool {
    cache_path(key).exists()
}

/// GET a URL into the cache slot; rejects non-PDF payloads — port of
/// _download_url (content-type must mention pdf or octet-stream).
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

/// Return the cached PDF path, downloading if needed — port of fetch()
/// source='auto': ADS OA_PDF resolver first, then arXiv fallback.
pub fn fetch(key: &str, eprint: &str, adsurl: &str) -> Option<PathBuf> {
    let path = cache_path(key);
    if path.exists() {
        return Some(path);
    }
    let bibcode = if adsurl.is_empty() {
        None
    } else {
        let t = adsurl.trim_end_matches('/');
        t.rsplit('/').next()
    };
    if let Some(bc) = bibcode {
        if let Some(url) = crate::ads::resolve_pdf_url(bc, "OA_PDF") {
            if let Some(p) = download_url(&path, &url) {
                return Some(p);
            }
        }
    }
    if !eprint.is_empty() {
        return download_url(&path, &format!("https://arxiv.org/pdf/{}", eprint.trim()));
    }
    None
}

/// Open cached PDFs with the platform opener.
pub fn open_paths(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        cmd.args(paths);
        let _ = cmd.spawn();
    }
    #[cfg(not(target_os = "macos"))]
    for p in paths {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}
