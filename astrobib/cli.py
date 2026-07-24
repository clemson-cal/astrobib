from __future__ import annotations

from pathlib import Path

import bibtexparser
import click
from bibtexparser.bparser import BibTexParser
from bibtexparser.customization import convert_to_unicode
from rich.console import Console
from rich.table import Table
from rich.text import Text

from .state import (
    get_token, set_token,
    get_library_path, STATE_FILE, PDF_CACHE_DIR, UAT_CACHE,
)
from .library import Library
from .keys import generate_key
from . import ads_client
from .export import export_refs

console = Console()


@click.group(invoke_without_command=True)
@click.pass_context
def main(ctx: click.Context):
    """astrobib — astrophysics literature manager."""
    if ctx.invoked_subcommand is None:
        from .tui.app import AstrobibApp
        AstrobibApp().run()


# ── config ────────────────────────────────────────────────────────────────────

@main.group("config", invoke_without_command=True)
@click.pass_context
def config_group(ctx: click.Context):
    """Show or edit astrobib configuration."""
    if ctx.invoked_subcommand is None:
        _show_config()


def _show_config() -> None:
    token = get_token()
    table = Table(box=None, show_header=False, padding=(0, 2, 0, 0))
    table.add_column("key", style="dim", width=16)
    table.add_column("value")
    table.add_column("source", style="dim")

    if token:
        masked = token[:4] + "…" + token[-4:] if len(token) > 8 else "****"
        table.add_row("ads_token", Text(masked, style="cyan"), _source("ADS_API_TOKEN", token))
    else:
        table.add_row("ads_token", Text("not set", style="yellow"), "")

    table.add_row("library", Text(str(get_library_path()), style="dim"), "")
    table.add_row("state_file", Text(str(STATE_FILE), style="dim"), "")
    table.add_row("pdf_cache", Text(str(PDF_CACHE_DIR), style="dim"), "")
    console.print(table)


def _source(env_var: str, current_value: str | None) -> str:
    import os
    return f"env:{env_var}" if os.environ.get(env_var) else "state.json"


@config_group.command("token")
@click.argument("value", required=False)
def config_token(value: str | None):
    """Get or set the ADS API token."""
    if value:
        set_token(value)
        console.print("[green]ADS token saved.[/green]")
        return
    current = get_token()
    if current:
        masked = current[:4] + "…" + current[-4:] if len(current) > 8 else "****"
        console.print(f"ads_token: [cyan]{masked}[/cyan]")
        if click.confirm("Replace?", default=False):
            set_token(click.prompt("New ADS token", hide_input=True))
            console.print("[green]ADS token saved.[/green]")
    else:
        console.print("[yellow]No ADS token set.[/yellow]")
        console.print("Get one at: https://ui.adsabs.harvard.edu/user/settings/token")
        set_token(click.prompt("ADS token", hide_input=True))
        console.print("[green]ADS token saved.[/green]")


# ── add ───────────────────────────────────────────────────────────────────────

def _open_merged():
    """Personal library merged with the active manuscript db (if any)."""
    from .library import MergedLibrary
    from .state import find_manuscript_db
    ms_root = find_manuscript_db()
    ms = Library(root=ms_root) if ms_root else None
    return MergedLibrary(personal=Library(root=get_library_path()), manuscript=ms)


def _write_targets(personal_only: bool = False, ms_only: bool = False
                   ) -> tuple[list[Library], str]:
    """Resolve write destinations: personal library and/or active manuscript db.

    Default (no flags): personal + manuscript when inside a manuscript repo,
    matching the TUI's import behavior.
    """
    from .state import find_manuscript_db
    if personal_only and ms_only:
        console.print("[red]--personal-only and --ms-only are mutually exclusive.[/red]")
        raise SystemExit(1)
    ms_root = find_manuscript_db()
    if ms_only:
        if ms_root is None:
            console.print("[red]Not inside a manuscript repo (no bib/ + .git found).[/red]")
            raise SystemExit(1)
        return [Library(root=ms_root)], f"manuscript db ({ms_root.name})"
    personal = Library(root=get_library_path())
    if personal_only or ms_root is None:
        return [personal], "personal library"
    return ([personal, Library(root=ms_root)],
            f"personal library + manuscript db ({ms_root.name})")


@main.command("add")
@click.argument("bibcode")
@click.option("--keywords", "-k", default="", metavar="KW[,KW]",
              help="Extra UAT keyword labels to append.")
@click.option("--force", "-f", is_flag=True, help="Overwrite existing entry without prompting.")
def add_cmd(bibcode: str, keywords: str, force: bool):
    """Add a paper by ADS bibcode.

    Inside a manuscript repo, writes to both the personal library and
    the manuscript database (like the TUI's import).
    """
    targets, dest = _write_targets()
    lib = targets[0]

    data = ads_client.fetch_bibtex(bibcode)
    if data is None:
        console.print(f"[red]Could not fetch BibTeX for {bibcode}[/red]")
        raise SystemExit(1)

    key = generate_key(data)

    if lib.has(key) and not force:
        existing = lib.get(key)
        if existing.data.get("adsurl") != data.get("adsurl"):
            console.print(f"\n[yellow]{key}[/yellow] already in library (different version).")
            console.print(f"  Current: {existing.title[:70]}")
            console.print(f"  New:     {data.get('title', '')[:70]}")
            if not click.confirm("  Replace with new version?", default=True):
                raise SystemExit(0)
        else:
            console.print(f"[yellow]{key} already in library. Use --force to overwrite.[/yellow]")
            raise SystemExit(1)

    if keywords:
        existing_kw = data.get("keywords", "")
        data["keywords"] = ", ".join(filter(None, [existing_kw, keywords]))

    entry = lib.save_entry(data)
    for extra in targets[1:]:
        extra.save_entry(dict(data))
    display = entry.short_key or entry.key
    suffix = f"  [dim]({entry.key})[/dim]" if display != entry.key else ""
    console.print(f"[green]Added[/green] {display}{suffix}  [dim]→ {dest}[/dim]")
    if entry.keywords:
        for kw in entry.keywords:
            console.print(f"  • {kw}")
    if entry.eprint:
        console.print(f"  arXiv: {entry.eprint}")


# ── import ────────────────────────────────────────────────────────────────────

def _ads_lookup(data: dict) -> tuple[dict | None, str]:
    """Resolve a foreign bib entry to a unique ADS record.

    Query preference: arXiv ID, then DOI (both unique identifiers), then
    exact title + first author + year, which must match exactly one record.
    Returns (canonical bibtex dict, "") on success, else (None, reason).
    """
    import re as _re
    eprint = (data.get("eprint") or "").strip()
    doi = (data.get("doi") or "").strip()
    needs_unique = False
    if eprint:
        ident = eprint if eprint.lower().startswith("arxiv:") else f"arXiv:{eprint}"
        query = f'identifier:"{ident}"'
    elif doi:
        query = f'doi:"{doi}"'
    else:
        title = _re.sub(r"[{}]", "", data.get("title") or "").replace('"', "").strip()
        last = (data.get("author") or "").split(" and ")[0].split(",")[0].strip()
        last = _re.sub(r"[{}\\]", "", last)
        year = (data.get("year") or "").strip()
        if not (title and last and year):
            return None, "not enough information for an unambiguous ADS query (need arXiv ID, DOI, or title+author+year)"
        query = f'title:"{title}" author:"^{last}" year:{year}'
        needs_unique = True
    try:
        results = ads_client.search(query, limit=2)
    except Exception as exc:
        return None, f"ADS lookup failed: {exc}"
    if not results:
        return None, f"no ADS match for {query}"
    if needs_unique and len(results) > 1:
        return None, f"ambiguous — multiple ADS matches for {query}"
    fetched = ads_client.fetch_bibtex(results[0].bibcode)
    if fetched is None:
        return None, f"could not fetch BibTeX for {results[0].bibcode}"
    return fetched, ""


@main.command("import")
@click.argument("file", type=click.Path(exists=True, path_type=Path))
@click.option("--personal-only", is_flag=True,
              help="Write only to the personal library.")
@click.option("--ms-only", is_flag=True,
              help="Write only to the active manuscript database.")
@click.option("--verify", is_flag=True,
              help="Prompt before replacing entries that are already present "
                   "(default: keep the existing entry and skip).")
def import_cmd(file: Path, personal_only: bool, ms_only: bool, verify: bool):
    """Import papers from a .bib file, resolving each against ADS.

    Every entry is resolved to a unique ADS record (by arXiv ID, DOI, or
    exact title+author+year) and imported with canonical ADS BibTeX and a
    regenerated cite key; entries that cannot be resolved unambiguously
    are skipped with a warning. Entries already present are skipped
    (--verify prompts to replace instead). Inside a manuscript repo,
    entries go to both the personal library and the manuscript database
    unless restricted by a flag. Prints copy-pasteable commands to update
    cite keys in your .tex files.
    """
    parser = BibTexParser(common_strings=True)
    parser.customization = convert_to_unicode
    with open(file) as f:
        bib = bibtexparser.load(f, parser=parser)

    if not bib.entries:
        console.print("[yellow]No entries found in file.[/yellow]")
        return

    targets, dest = _write_targets(personal_only, ms_only)
    console.print(f"Importing into {dest}\n")
    added = skipped = 0
    renames: list[tuple[str, str]] = []

    for data in bib.entries:
        orig_key = data.get("ID", "?")
        resolved, reason = _ads_lookup(data)
        if resolved is None:
            console.print(f"  [yellow]⚠ {orig_key} skipped — {reason}[/yellow]")
            skipped += 1
            continue
        data = resolved
        key = generate_key(data)
        holder = next((t for t in targets if t.has(key)), None)
        if holder is not None:
            replace = False
            if verify:
                existing = holder.get(key)
                console.print(f"\n[yellow]{key}[/yellow] already present:")
                console.print(f"  Have:   {existing.title[:65]}")
                console.print(f"  Import: {data.get('title', '')[:65]}")
                replace = click.confirm("  Replace?", default=False)
            if not replace:
                console.print(f"  [dim]{orig_key} → {key}  (already present — kept existing)[/dim]")
                if orig_key != key:
                    renames.append((orig_key, key))  # key mapping still applies
                skipped += 1
                continue
        for t in targets:
            entry = t.save_entry(dict(data))
        if orig_key != entry.key:
            renames.append((orig_key, entry.key))
            console.print(f"  {orig_key} → [cyan]{entry.key}[/cyan]")
        else:
            console.print(f"  [cyan]{entry.key}[/cyan]")
        added += 1

    console.print(f"\n[green]{added}[/green] imported → {dest}, [dim]{skipped}[/dim] skipped.")

    if renames:
        from .state import find_manuscript_db
        ms_root = find_manuscript_db()
        tex_files = sorted(ms_root.glob("*.tex")) if ms_root else []
        tex_arg = " ".join(f.name for f in tex_files) or "main.tex"
        console.print("\nCite key replacements (copy/paste to update your TeX source):")
        for old, new in renames:
            console.print(f"  perl -pi -e 's/\\b\\Q{old}\\E\\b/{new}/g' {tex_arg}",
                          highlight=False, markup=False)


# ── export ────────────────────────────────────────────────────────────────────

@main.command("export")
@click.argument("tex_files", nargs=-1, type=click.Path(exists=True, path_type=Path))
@click.option("--output", "-o", default="refs.bib", show_default=True,
              type=click.Path(path_type=Path))
@click.option("--list-missing", is_flag=True)
def export_cmd(tex_files: tuple[Path, ...], output: Path, list_missing: bool):
    """Generate refs.bib by scanning TeX source for cite keys."""
    from .export import expand_tex_sources
    library = _open_merged()
    paths = expand_tex_sources(
        list(tex_files) or sorted(Path.cwd().glob("*.tex")), Path.cwd())
    if not paths:
        console.print("[red]No .tex files found.[/red]")
        raise SystemExit(1)

    console.print(f"Scanning {len(paths)} file(s)…")
    found, missing = export_refs(paths, output, library)
    console.print(f"[green]Wrote {len(found)} entr{'y' if len(found)==1 else 'ies'} → {output}[/green]")
    if missing:
        console.print(f"[yellow]Missing {len(missing)} key(s):[/yellow]")
        for key in missing:
            console.print(f"  [yellow]{key}[/yellow]")
        if list_missing:
            raise SystemExit(1)


# ── refs (manuscript sync) ────────────────────────────────────────────────────

@main.command("refs")
@click.argument("tex_files", nargs=-1, type=click.Path(exists=True, path_type=Path))
@click.option("--output", "-o", default="refs.bib", show_default=True,
              type=click.Path(path_type=Path))
@click.option("--prune", is_flag=True,
              help="Remove uncited entries from the manuscript database.")
def refs_cmd(tex_files: tuple[Path, ...], output: Path, prune: bool):
    """Sync the manuscript database with cited keys, then write refs.bib.

    Run inside a manuscript repo (a directory containing bib/ and .git).
    Cited keys missing from the manuscript database are copied in from
    the personal library; keys found nowhere are reported. refs.bib is
    generated from the manuscript database only.
    """
    from .export import expand_tex_sources, manuscript_tex_files, scan_tex_files
    from .library import MergedLibrary
    from .state import find_manuscript_db

    ms_root = find_manuscript_db()
    if ms_root is None:
        console.print("[red]Not inside a manuscript repo (no bib/ + .git found).[/red]")
        raise SystemExit(1)

    merged = MergedLibrary(personal=Library(root=get_library_path()),
                           manuscript=Library(root=ms_root))
    ms = merged.manuscript

    paths = (expand_tex_sources(list(tex_files), ms_root) if tex_files
             else manuscript_tex_files(ms_root))
    if not paths:
        console.print(f"[red]No .tex files found in {ms_root}.[/red]")
        raise SystemExit(1)

    console.print(f"Manuscript db: [cyan]{ms_root}[/cyan]")
    console.print(f"Scanning {len(paths)} file(s)…")
    keys = scan_tex_files(paths)

    copied: list[str] = []
    missing: list[str] = []
    for key in sorted(keys):
        if ms.has(key):
            continue
        if merged.personal.has(key):
            merged.add_to_manuscript(key)
            copied.append(key)
        else:
            missing.append(key)

    uncited = sorted(e.key for e in ms.entries() if e.key not in keys)
    if prune and uncited:
        for key in uncited:
            ms.remove_entry(key)
        console.print(f"[yellow]Pruned {len(uncited)} uncited entr{'y' if len(uncited)==1 else 'ies'}.[/yellow]")
        uncited = []

    output_path = output if output.is_absolute() else ms_root / output
    found, _ = export_refs(paths, output_path, ms)

    if copied:
        console.print(f"[green]Copied {len(copied)} entr{'y' if len(copied)==1 else 'ies'} from personal library:[/green]")
        for key in copied:
            console.print(f"  [green]{key}[/green]")
    console.print(f"[green]Wrote {len(found)} entr{'y' if len(found)==1 else 'ies'} → {output_path}[/green]")
    if uncited:
        console.print(f"[dim]{len(uncited)} uncited entr{'y' if len(uncited)==1 else 'ies'} in bib/ (use --prune to remove):[/dim]")
        for key in uncited:
            console.print(f"  [dim]{key}[/dim]")
    if missing:
        console.print(f"[yellow]Missing {len(missing)} key(s) (not in any database):[/yellow]")
        for key in missing:
            console.print(f"  [yellow]{key}[/yellow]")
        raise SystemExit(1)


# ── search ────────────────────────────────────────────────────────────────────

@main.command("search")
@click.argument("query")
@click.option("--limit", "-n", default=10, show_default=True)
@click.option("--ads", "use_ads", is_flag=True, help="Search ADS instead of local library.")
def search_cmd(query: str, limit: int, use_ads: bool):
    """Search the local library or ADS."""
    if use_ads:
        _search_ads(query, limit)
    else:
        _search_local(query, limit)


# ── show ──────────────────────────────────────────────────────────────────────

@main.command("show")
@click.argument("key")
def show_cmd(key: str):
    """Print the BibTeX entry for a cite key (full or shortened)."""
    library = _open_merged()
    entry = library.resolve(key)
    if entry is None:
        matches = library.possible_matches(key)
        if len(matches) > 1:
            console.print(f"[yellow]Ambiguous key '{key}' — did you mean:[/yellow]")
            for m in matches[:6]:
                console.print(f"  [cyan]{m.short_key}[/cyan]  [dim]{m.title[:55]}[/dim]")
        else:
            console.print(f"[red]{key} not found.[/red]")
        raise SystemExit(1)
    locs = []
    if library.in_personal(entry.key):
        locs.append("personal")
    if library.in_manuscript(entry.key):
        locs.append(f"◆ ms: {library.manuscript_root.name}")
    console.print(f"[dim]% in: {' · '.join(locs)}[/dim]")
    console.print(entry.path.read_text())


# ── pdf ───────────────────────────────────────────────────────────────────────

_SOURCE = click.Choice(["auto", "arxiv", "browser"])
_SOURCE_OPT = click.option(
    "--source", "-s", type=_SOURCE, default="auto", show_default=True,
    help="auto: ADS OA then arXiv; arxiv: arXiv only; browser: open browser + watch Downloads.",
)


@main.group("pdf")
def pdf_group():
    """Manage locally cached PDFs for a paper."""


def _resolve(key_or_bibcode: str) -> tuple[str, str, str, str]:
    """Return (full_key, eprint, doi, adsurl), querying ADS if not in library."""
    library = _open_merged()
    entry = library.resolve(key_or_bibcode) or library.get_by_bibcode(key_or_bibcode)
    if entry:
        return entry.key, entry.eprint, entry.doi, entry.adsurl
    matches = library.possible_matches(key_or_bibcode)
    if len(matches) > 1:
        console.print(f"[yellow]Ambiguous key '{key_or_bibcode}' — did you mean:[/yellow]")
        for m in matches[:6]:
            console.print(f"  [cyan]{m.short_key}[/cyan]  [dim]{m.title[:55]}[/dim]")
        raise SystemExit(1)
    console.print(f"[dim]{key_or_bibcode} not in library — querying ADS…[/dim]")
    try:
        from . import ads_client
        results = ads_client.search(f"bibcode:{key_or_bibcode}", limit=1)
        if not results:
            console.print(f"[red]{key_or_bibcode} not found on ADS.[/red]")
            raise SystemExit(1)
        a = results[0]
        adsurl = f"https://ui.adsabs.harvard.edu/abs/{a.bibcode}"
        return key_or_bibcode, ads_client.arxiv_id_from_article(a) or "", (a.doi or [""])[0], adsurl
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)


@pdf_group.command("check")
@click.argument("citekey_or_bibcode")
def pdf_check(citekey_or_bibcode: str):
    """Show available PDF sources without downloading."""
    from . import pdf
    key, eprint, doi, adsurl = _resolve(citekey_or_bibcode)
    cached_path = pdf.cache_path(key)
    lib = _open_merged()
    entry = lib.get(key)
    display = (entry.short_key or key) if entry else key
    suffix = f"  [dim]({key})[/dim]" if display != key else ""
    console.print(f"\n[bold]{display}[/bold]{suffix}\n")

    # cached
    if cached_path.exists():
        sz = cached_path.stat().st_size // 1024
        console.print(f"  [green]cached[/green]     {cached_path}  ({sz} KB)")
    else:
        console.print(f"  [dim]cached     —[/dim]")

    # arXiv
    if eprint:
        console.print(f"  [cyan]arXiv[/cyan]      {eprint}  [green]✓ auto[/green]")
        console.print(f"  [dim]           → https://arxiv.org/pdf/{eprint}[/dim]")
    else:
        console.print(f"  [dim]arXiv      —[/dim]")

    # ADS OA resolver
    bibcode = adsurl.rstrip("/").rsplit("/", 1)[-1] if adsurl else None
    if bibcode:
        if not get_token():
            console.print(f"  [dim]ADS OA     — (no token)[/dim]")
        else:
            oa = ads_client.resolve_pdf_url(bibcode, "OA_PDF")
            if oa:
                oa_display = oa if len(oa) <= 63 else oa[:60] + "…"
                console.print(f"  [cyan]ADS OA[/cyan]     {bibcode}  [green]✓ auto[/green]")
                console.print(f"  [dim]           → {oa_display}[/dim]")
            else:
                console.print(f"  [dim]ADS OA     {bibcode}  ✗ no OA PDF[/dim]")
    else:
        console.print(f"  [dim]ADS OA     —[/dim]")

    # browser
    browser_url = pdf.browser_open_url(doi=doi, adsurl=adsurl)
    if browser_url:
        console.print(f"  [yellow]browser[/yellow]    requires action"
                      f"  [dim]→ astrobib pdf download {display} --source browser[/dim]")
    else:
        console.print(f"  [dim]browser    — (no DOI or ADS URL)[/dim]")


@pdf_group.command("download")
@click.argument("citekey_or_bibcode")
@_SOURCE_OPT
def pdf_download(citekey_or_bibcode: str, source: str):
    """Download PDF, replacing any cached copy."""
    from . import pdf
    key, eprint, doi, adsurl = _resolve(citekey_or_bibcode)
    lib = _open_merged()
    entry = lib.get(key)
    display = (entry.short_key or key) if entry else key

    if source == "browser":
        url = pdf.browser_open_url(doi=doi, adsurl=adsurl)
        if not url:
            console.print(f"[red]No DOI or ADS URL for {key}.[/red]")
            raise SystemExit(1)
        before = pdf.downloads_snapshot()
        console.print(f"Opening browser: {url}")
        pdf.browser_open(url)
        console.print(f"[dim]Watching ~/Downloads for a new PDF (up to 60s)…[/dim]")
        path = pdf.poll_downloads(key, before, timeout=60)
        if path is None:
            console.print(f"[red]No PDF appeared in ~/Downloads within 60s.[/red]")
            raise SystemExit(1)
        sz = path.stat().st_size // 1024
        console.print(f"[green]Saved {path}  ({sz} KB)[/green]")
        return

    if not eprint and not doi:
        console.print(f"[red]No arXiv ID or DOI for {key}.[/red]")
        raise SystemExit(1)
    console.print(f"Downloading {display} (source={source})…")
    path = pdf.fetch(key, eprint=eprint, doi=doi, adsurl=adsurl, source=source, force=True)
    if path is None:
        console.print(f"[red]No PDF found via source={source}.[/red]")
        browser_url = pdf.browser_open_url(doi=doi, adsurl=adsurl)
        if browser_url and source == "auto":
            console.print(f"[dim]→ try: astrobib pdf download {display} --source browser[/dim]")
        raise SystemExit(1)
    sz = path.stat().st_size // 1024
    console.print(f"[green]Saved {path}  ({sz} KB)[/green]")


@pdf_group.command("open")
@click.argument("citekey_or_bibcode")
@click.option("--source", "-s", type=click.Choice(["auto", "arxiv"]), default="auto",
              show_default=True, help="auto: ADS OA then arXiv; arxiv: arXiv only.")
def pdf_open(citekey_or_bibcode: str, source: str):
    """Open PDF, downloading if not cached."""
    from . import pdf
    key, eprint, doi, adsurl = _resolve(citekey_or_bibcode)
    lib = _open_merged()
    entry = lib.get(key)
    display = (entry.short_key or key) if entry else key
    if not eprint and not doi:
        console.print(f"[red]No arXiv ID or DOI for {key}.[/red]")
        raise SystemExit(1)
    force = source != "auto"
    if pdf.is_cached(key) and not force:
        console.print(f"Opening cached {display}…")
    elif doi and source != "arxiv":
        console.print(f"Fetching {display} via ADS OA resolver…")
    else:
        console.print(f"Fetching {display} from arXiv…")
    ok = pdf.open_pdf(key, eprint=eprint, doi=doi, adsurl=adsurl, source=source, force=force)
    if not ok:
        console.print(f"[red]No PDF found via source={source}.[/red]")
        browser_url = pdf.browser_open_url(doi=doi, adsurl=adsurl)
        if browser_url:
            console.print(f"[dim]→ try: astrobib pdf download {display} --source browser[/dim]")
        raise SystemExit(1)


@pdf_group.command("clear")
@click.argument("citekey_or_bibcode")
def pdf_clear(citekey_or_bibcode: str):
    """Remove the locally cached PDF."""
    from . import pdf
    key, _, _, _ = _resolve(citekey_or_bibcode)
    path = pdf.cache_path(key)
    if not path.exists():
        console.print(f"[dim]No cached PDF for {key}.[/dim]")
        return
    path.unlink()
    console.print(f"[green]Cleared {path}[/green]")


# ── list ──────────────────────────────────────────────────────────────────────

@main.command("list")
@click.option("--keyword", "-k", default="", help="Filter by UAT keyword label.")
def list_cmd(keyword: str):
    """List papers in the personal library and active manuscript db."""
    library = _open_merged()
    if keyword:
        from .uat import get_uat
        uat = get_uat(UAT_CACHE, auto_fetch=False)
        desc = uat.descendant_labels(keyword) if uat else {keyword}
        entries = library.by_keyword(keyword, descendant_labels=desc)
    else:
        entries = library.entries()
    entries = sorted(entries, key=lambda e: e.year, reverse=True)
    _print_entry_table(entries, library)
    summary = f"\n{len(entries)} paper(s)"
    if library.manuscript is not None:
        n_ms = sum(1 for e in entries if library.in_manuscript(e.key))
        summary += f" · [cyan]◆ {n_ms} in ms: {library.manuscript_root.name}[/cyan]"
    console.print(summary)


# ── quota ─────────────────────────────────────────────────────────────────────

@main.command("quota")
def quota_cmd():
    """Show ADS API rate-limit usage."""
    import datetime
    console.print("Checking ADS quota…")
    quota = ads_client.refresh_quota()
    if quota is None:
        console.print("[red]Could not reach ADS or no token configured.[/red]")
        raise SystemExit(1)

    remaining = quota["remaining"]
    limit = quota["limit"]
    used = limit - remaining
    reset_ts = quota["reset"]
    reset_str = (
        __import__("datetime").datetime.fromtimestamp(reset_ts).strftime("%Y-%m-%d %H:%M")
        if reset_ts else "unknown"
    )

    bar_width = 30
    filled = int(bar_width * remaining / limit) if limit else 0
    bar = "█" * filled + "░" * (bar_width - filled)
    color = "green" if remaining > limit * 0.2 else "yellow" if remaining > 0 else "red"
    console.print(f"  [{color}]{bar}[/{color}]  {remaining}/{limit} remaining")
    console.print(f"  {used} calls used today · resets {reset_str}")


# ── keywords ──────────────────────────────────────────────────────────────────

@main.command("keywords")
def keywords_cmd():
    """List all unique keywords across the library."""
    library = _open_merged()
    for kw in library.all_keywords():
        count = len(library.by_keyword(kw))
        console.print(f"  [cyan]{kw}[/cyan] [dim]({count})[/dim]")


# ── uat ───────────────────────────────────────────────────────────────────────

@main.group("uat")
def uat_group():
    """Manage the local UAT cache."""


@uat_group.command("browse")
def uat_browse():
    """Interactively browse the UAT concept hierarchy."""
    from .uat import get_uat
    from .tui.uat_browser import UATBrowserApp
    uat = get_uat(UAT_CACHE)
    if uat is None:
        console.print("[red]UAT not cached. Run: astrobib uat update[/red]")
        raise SystemExit(1)
    UATBrowserApp(uat).run()


@uat_group.command("update")
def uat_update():
    """Download/refresh the UAT from GitHub."""
    from .uat import UAT, UAT_URL
    UAT_CACHE.parent.mkdir(parents=True, exist_ok=True)
    console.print(f"Fetching UAT from {UAT_URL} …")
    try:
        uat = UAT.fetch_and_cache(UAT_CACHE)
        console.print(f"[green]Cached {len(uat)} UAT concepts → {UAT_CACHE}[/green]")
    except Exception as e:
        console.print(f"[red]Failed: {e}[/red]")
        raise SystemExit(1)


@uat_group.command("search")
@click.argument("query")
@click.option("--limit", "-n", default=15, show_default=True)
def uat_search(query: str, limit: int):
    """Search UAT concept labels."""
    from .uat import get_uat
    uat = get_uat(UAT_CACHE)
    if uat is None:
        console.print("[red]UAT not cached. Run: astrobib uat update[/red]")
        raise SystemExit(1)
    results = uat.search(query, limit=limit)
    if not results:
        console.print("[dim]No matches.[/dim]")
        return
    for c in results:
        parents = uat.parents(c.uid)
        breadcrumb = " › ".join(p.label for p in parents[-2:]) + (" › " if parents else "") + c.label
        console.print(f"  [cyan]{breadcrumb}[/cyan]  [dim](UID {c.uid})[/dim]")


@uat_group.command("show")
@click.argument("label")
def uat_show(label: str):
    """Show a UAT concept and its immediate children."""
    from .uat import get_uat
    uat = get_uat(UAT_CACHE)
    if uat is None:
        console.print("[red]UAT not cached. Run: astrobib uat update[/red]")
        raise SystemExit(1)
    concept = uat.by_label(label)
    if concept is None:
        console.print(f"[red]'{label}' not found in UAT.[/red]")
        raise SystemExit(1)
    parents = uat.parents(concept.uid)
    if parents:
        console.print(f"[dim]broader:[/dim] {' › '.join(p.label for p in parents)}")
    console.print(f"[bold cyan]{concept.label}[/bold cyan]  [dim](UID {concept.uid})[/dim]")
    if concept.definition:
        console.print(f"[dim]{concept.definition[:200]}[/dim]")
    children = uat.children(concept.uid)
    if children:
        console.print(f"\n[dim]narrower ({len(children)}):[/dim]")
        for child in children:
            console.print(f"  • {child.label}")


# ── helpers ───────────────────────────────────────────────────────────────────

def _search_local(query: str, limit: int):
    library = _open_merged()
    q = query.lower()
    results = [
        e for e in library.entries()
        if q in e.title.lower() or q in e.author.lower() or q in e.key.lower()
        or any(q in kw.lower() for kw in e.keywords)
    ]
    results = sorted(results, key=lambda e: e.year, reverse=True)[:limit]
    _print_entry_table(results, library)


def _search_ads(query: str, limit: int):
    try:
        articles = ads_client.search(query, limit=limit)
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)
    table = Table("Bibcode", "Year", "First Author", "Title", box=None,
                  show_header=True, header_style="bold")
    for a in articles:
        first_author = a.author[0].split(",")[0] if a.author else ""
        title = a.title[0] if a.title else ""
        table.add_row(
            Text(a.bibcode, style="cyan"),
            str(a.year),
            first_author,
            title[:70] + ("…" if len(title) > 70 else ""),
        )
    console.print(table)
    console.print(f"\n{len(articles)} result(s) — use [cyan]astrobib add <bibcode>[/cyan] to add")


def _print_entry_table(entries: list, library=None):
    """Entry table with the TUI's indicator columns: ↓ cached, ◆ in
    manuscript db, ★ starred."""
    from . import pdf as _pdf
    if not entries:
        console.print("[dim]No results.[/dim]")
        return
    has_ms = library is not None and getattr(library, "manuscript", None) is not None
    cols = ["↓", "◆", "★"] if has_ms else ["↓", "★"]
    table = Table(*cols, "Key", "Year", "First Author", "Title", box=None,
                  show_header=True, header_style="bold")
    for e in entries:
        display = e.short_key or e.key
        row = ["↓" if _pdf.is_cached(e.key) else ""]
        if has_ms:
            row.append("◆" if library.in_manuscript(e.key) else "")
        row.append("★" if e.starred else "")
        table.add_row(
            *row,
            Text(display, style="cyan"),
            e.year,
            e.first_author_last,
            e.title[:70] + ("…" if len(e.title) > 70 else ""),
        )
    console.print(table)
