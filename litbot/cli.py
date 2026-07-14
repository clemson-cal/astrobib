from __future__ import annotations

from pathlib import Path

import bibtexparser
import click
from bibtexparser.bparser import BibTexParser
from bibtexparser.customization import convert_to_unicode
from rich.console import Console
from rich.table import Table
from rich.text import Text

from .state import get_token, set_token, get_library_path, UAT_CACHE
from .library import Library
from .keys import generate_key
from . import ads_client
from .export import export_refs

console = Console()


@click.group(invoke_without_command=True)
@click.pass_context
def main(ctx: click.Context):
    """litbot — astrophysics literature manager."""
    if ctx.invoked_subcommand is None:
        from .tui.app import LitbotApp
        LitbotApp().run()


# ── token ─────────────────────────────────────────────────────────────────────

@main.command("token")
@click.argument("token", required=False)
def token_cmd(token: str | None):
    """Set or show the ADS API token.

    With no argument, shows current token (masked) or prompts to enter one.
    """
    if token:
        set_token(token)
        console.print("[green]ADS token saved.[/green]")
        return

    current = get_token()
    if current:
        masked = current[:4] + "…" + current[-4:] if len(current) > 8 else "****"
        console.print(f"ADS token: [dim]{masked}[/dim]")
        if click.confirm("Replace?", default=False):
            new_token = click.prompt("New ADS token", hide_input=True)
            set_token(new_token)
            console.print("[green]ADS token saved.[/green]")
    else:
        console.print("[yellow]No ADS token set.[/yellow]")
        console.print("Get one at: https://ui.adsabs.harvard.edu/user/settings/token")
        new_token = click.prompt("ADS token", hide_input=True)
        set_token(new_token)
        console.print("[green]ADS token saved.[/green]")


# ── add ───────────────────────────────────────────────────────────────────────

@main.command("add")
@click.argument("bibcode")
@click.option("--keywords", "-k", default="", metavar="KW[,KW]",
              help="Extra UAT keyword labels to append.")
@click.option("--force", "-f", is_flag=True, help="Overwrite existing entry without prompting.")
def add_cmd(bibcode: str, keywords: str, force: bool):
    """Add a paper to the library by ADS bibcode."""
    lib = Library(root=get_library_path())

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
    console.print(f"[green]Added[/green] {entry.key}")
    if entry.keywords:
        for kw in entry.keywords:
            console.print(f"  • {kw}")
    if entry.eprint:
        console.print(f"  arXiv: {entry.eprint}")


# ── import ────────────────────────────────────────────────────────────────────

@main.command("import")
@click.argument("file", type=click.Path(exists=True, path_type=Path))
def import_cmd(file: Path):
    """Import papers from a .bib file into the local library."""
    parser = BibTexParser(common_strings=True)
    parser.customization = convert_to_unicode
    with open(file) as f:
        bib = bibtexparser.load(f, parser=parser)

    if not bib.entries:
        console.print("[yellow]No entries found in file.[/yellow]")
        return

    lib = Library(root=get_library_path())
    added = skipped = 0

    for data in bib.entries:
        key = generate_key(data)
        if lib.has(key):
            existing = lib.get(key)
            console.print(f"\n[yellow]{key}[/yellow] already in library:")
            console.print(f"  Have:   {existing.title[:65]}")
            console.print(f"  Import: {data.get('title', '')[:65]}")
            if click.confirm("  Replace?", default=False):
                lib.save_entry(data)
                added += 1
            else:
                skipped += 1
        else:
            lib.save_entry(data)
            added += 1

    console.print(f"\n[green]{added}[/green] imported, [dim]{skipped}[/dim] skipped.")


# ── export ────────────────────────────────────────────────────────────────────

@main.command("export")
@click.argument("tex_files", nargs=-1, type=click.Path(exists=True, path_type=Path))
@click.option("--output", "-o", default="refs.bib", show_default=True,
              type=click.Path(path_type=Path))
@click.option("--list-missing", is_flag=True)
def export_cmd(tex_files: tuple[Path, ...], output: Path, list_missing: bool):
    """Generate refs.bib by scanning TeX source for cite keys."""
    library = Library(root=get_library_path())
    paths = list(tex_files) or sorted(Path.cwd().glob("*.tex"))
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
    """Print the BibTeX entry for a cite key."""
    library = Library(root=get_library_path())
    entry = library.get(key)
    if entry is None:
        console.print(f"[red]{key} not found.[/red]")
        raise SystemExit(1)
    console.print(entry.path.read_text())


# ── open ──────────────────────────────────────────────────────────────────────

@main.command("open")
@click.argument("key")
def open_cmd(key: str):
    """Open the PDF for a cite key (fetching from arXiv if needed)."""
    from . import pdf
    library = Library(root=get_library_path())
    entry = library.get(key)
    if entry is None:
        console.print(f"[red]{key} not found.[/red]")
        raise SystemExit(1)
    if not entry.eprint and not entry.doi:
        console.print(f"[yellow]No arXiv ID or DOI for {key}.[/yellow]")
        raise SystemExit(1)
    if pdf.is_cached(key):
        console.print("Opening cached PDF…")
    elif entry.doi:
        console.print("Fetching via Unpaywall…")
    else:
        console.print(f"Fetching from arXiv:{entry.eprint}…")
    if not pdf.open_pdf(key, eprint=entry.eprint, doi=entry.doi):
        console.print(f"[red]No open-access PDF found.[/red]")
        raise SystemExit(1)


# ── list ──────────────────────────────────────────────────────────────────────

@main.command("list")
@click.option("--keyword", "-k", default="", help="Filter by UAT keyword label.")
def list_cmd(keyword: str):
    """List papers in the library."""
    library = Library(root=get_library_path())
    if keyword:
        from .uat import get_uat
        uat = get_uat(UAT_CACHE, auto_fetch=False)
        desc = uat.descendant_labels(keyword) if uat else {keyword}
        entries = library.by_keyword(keyword, descendant_labels=desc)
    else:
        entries = library.entries()
    entries = sorted(entries, key=lambda e: e.year, reverse=True)
    _print_entry_table(entries)
    console.print(f"\n{len(entries)} paper(s)")


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
    library = Library(root=get_library_path())
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
        console.print("[red]UAT not cached. Run: litbot uat update[/red]")
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
        console.print("[red]UAT not cached. Run: litbot uat update[/red]")
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
        console.print("[red]UAT not cached. Run: litbot uat update[/red]")
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
    library = Library(root=get_library_path())
    q = query.lower()
    results = [
        e for e in library.entries()
        if q in e.title.lower() or q in e.author.lower() or q in e.key.lower()
        or any(q in kw.lower() for kw in e.keywords)
    ]
    results = sorted(results, key=lambda e: e.year, reverse=True)[:limit]
    _print_entry_table(results)


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
    console.print(f"\n{len(articles)} result(s) — use [cyan]litbot add <bibcode>[/cyan] to add")


def _print_entry_table(entries: list):
    if not entries:
        console.print("[dim]No results.[/dim]")
        return
    table = Table("Key", "Year", "First Author", "Title", box=None,
                  show_header=True, header_style="bold")
    for e in entries:
        table.add_row(
            Text(e.key, style="cyan"),
            e.year,
            e.first_author_last,
            e.title[:70] + ("…" if len(e.title) > 70 else ""),
        )
    console.print(table)
