from __future__ import annotations

from pathlib import Path

import click
from rich.console import Console
from rich.table import Table
from rich.text import Text

from .config import get_config, save_database, DEFAULT_DB_DIR, UAT_CACHE
from .library import Library, MergedLibrary
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


# ── db ────────────────────────────────────────────────────────────────────────

@main.group("db")
def db_group():
    """Manage the bib database."""


@db_group.command("clone")
@click.argument("url")
@click.option("--name", "-n", default="", help="Name for this database (default: repo name).")
@click.option("--dest", "-d", type=click.Path(path_type=Path),
              help="Where to clone. Defaults to ~/.local/share/litbot/databases/<name>.")
def db_clone(url: str, name: str, dest: Path | None):
    """Clone a remote bib database and register it."""
    from . import db as dbmod

    repo_name = url.rstrip("/").rsplit("/", 1)[-1].removesuffix(".git")
    db_name = name or repo_name
    target = dest or DEFAULT_DB_DIR / db_name
    if target.exists():
        console.print(f"[red]{target} already exists. Use --dest or remove it first.[/red]")
        raise SystemExit(1)

    console.print(f"Cloning {url} → {target}…")
    try:
        dbmod.clone(url, target)
    except dbmod.DatabaseError as e:
        console.print(f"[red]Clone failed: {e}[/red]")
        raise SystemExit(1)

    save_database(db_name, target)
    console.print(f"[green]Cloned and registered '{db_name}'[/green]")


@db_group.command("init")
@click.argument("path", default="", required=False)
@click.option("--name", "-n", default="", help="Name for this database (default: directory name).")
def db_init(path: str, name: str):
    """Create a new empty bib database and register it.

    PATH defaults to ~/.local/share/litbot/databases/<name>.
    If PATH already exists and contains a bib/ directory, it is registered as-is.
    """
    from . import db as dbmod

    if not path and not name:
        console.print("[red]Provide a path or --name.[/red]")
        raise SystemExit(1)

    target = Path(path).expanduser().resolve() if path else DEFAULT_DB_DIR / name
    db_name = name or target.name

    if target.exists() and (target / "bib").is_dir():
        save_database(db_name, target)
        console.print(f"[green]Registered '{db_name}' → {target}[/green]")
        return

    if target.exists() and any(target.iterdir()):
        console.print(f"[red]{target} already exists and is not empty.[/red]")
        raise SystemExit(1)

    console.print(f"Creating new database '{db_name}' at {target}…")
    dbmod.init_empty(target)
    save_database(db_name, target)
    console.print(f"[green]Created and registered '{db_name}'[/green]")


@db_group.command("list")
def db_list():
    """List all configured databases."""
    config = _get_config()
    for name, path in config.databases.items():
        default_marker = " [green](default)[/green]" if name == config.default_database else ""
        exists = " [red](missing)[/red]" if not path.exists() else ""
        console.print(f"  [cyan]{name}[/cyan]{default_marker}  {path}{exists}")


@db_group.command("pull")
@click.option("--db", "db_name", default="", help="Database to pull (default: default database).")
def db_pull(db_name: str):
    """Pull latest changes from the remote (default database unless --db given)."""
    from . import db as dbmod
    config = _get_config()
    try:
        db_path = config.db_path(db_name or None)
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)
    console.print(f"Pulling '{db_name or config.default_database}' ({db_path})…")
    try:
        out = dbmod.pull(db_path)
        console.print(out or "Already up to date.")
    except dbmod.DatabaseError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)


@db_group.command("push")
@click.option("--db", "db_name", default="", help="Database to push (default: default database).")
def db_push(db_name: str):
    """Push committed changes to the remote."""
    from . import db as dbmod
    config = _get_config()
    try:
        db_path = config.db_path(db_name or None)
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)
    try:
        out = dbmod.push(db_path)
        console.print(f"[green]{out}[/green]")
    except dbmod.DatabaseError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)


@db_group.command("publish")
@click.option("--message", "-m", default="Update library", show_default=True)
@click.option("--db", "db_name", default="", help="Database to publish (default: default database).")
def db_publish(message: str, db_name: str):
    """Stage all changes, commit, and push (default database unless --db given)."""
    from . import db as dbmod
    config = _get_config()
    try:
        db_path = config.db_path(db_name or None)
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)
    try:
        out = dbmod.publish(db_path, message=message)
        console.print(f"[green]{out}[/green]")
    except dbmod.DatabaseError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)


@db_group.command("status")
@click.option("--db", "db_name", default="", help="Database to check (default: all).")
def db_status(db_name: str):
    """Show pending changes. Checks all databases unless --db given."""
    from . import db as dbmod
    config = _get_config()
    targets = (
        {db_name: config.databases[db_name]}
        if db_name
        else config.databases
    )
    for name, path in targets.items():
        default_marker = " (default)" if name == config.default_database else ""
        try:
            url = dbmod.remote_url(path)
            console.print(f"[bold]{name}[/bold]{default_marker}  [dim]{url}[/dim]")
        except Exception:
            console.print(f"[bold]{name}[/bold]{default_marker}  [dim]{path}[/dim]")
        pending = dbmod.status(path)
        console.print(pending if pending else "[dim]  Clean.[/dim]")


@db_group.command("log")
@click.option("--n", "-n", default=10, show_default=True)
@click.option("--db", "db_name", default="", help="Database (default: default database).")
def db_log(n: int, db_name: str):
    """Show recent commits."""
    from . import db as dbmod
    config = _get_config()
    try:
        db_path = config.db_path(db_name or None)
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)
    console.print(dbmod.log(db_path, n=n))


# ── export ────────────────────────────────────────────────────────────────────

@main.command("export")
@click.argument("tex_files", nargs=-1, type=click.Path(exists=True, path_type=Path))
@click.option("--output", "-o", default="refs.bib", show_default=True,
              type=click.Path(path_type=Path))
@click.option("--list-missing", is_flag=True)
def export_cmd(tex_files: tuple[Path, ...], output: Path, list_missing: bool):
    """Generate refs.bib by scanning TeX source for cite keys."""
    library = _load_merged()
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


# ── add ───────────────────────────────────────────────────────────────────────

@main.command("add")
@click.argument("bibcode")
@click.option("--keywords", "-k", default="", metavar="KW[,KW]",
              help="Extra UAT keyword labels to append.")
@click.option("--db", "db_name", default="", help="Database to write to (default: default database).")
@click.option("--force", "-f", is_flag=True, help="Overwrite existing entry.")
def add_cmd(bibcode: str, keywords: str, db_name: str, force: bool):
    """Add a paper to the library by ADS bibcode."""
    config = _get_config()
    try:
        db_path = config.db_path(db_name or None)
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)

    library = Library(root=db_path)
    merged = _load_merged()
    data = ads_client.fetch_bibtex(bibcode)
    if data is None:
        console.print(f"[red]Could not fetch BibTeX for {bibcode}[/red]")
        raise SystemExit(1)

    key = data["ID"]
    if merged.has(key) and not force:
        existing = merged.get(key)
        console.print(f"[yellow]{key} already in library ({existing.path}). Use --force to overwrite.[/yellow]")
        raise SystemExit(1)

    if keywords:
        existing_kw = data.get("keywords", "")
        data["keywords"] = ", ".join(filter(None, [existing_kw, keywords]))

    entry = library.save_entry(data)
    target_name = db_name or config.default_database

    from . import db as dbmod
    try:
        dbmod.commit_entry(db_path, entry.key)
    except Exception:
        pass

    console.print(f"[green]Added[/green] {entry.key} → '{target_name}'")
    if entry.keywords:
        for kw in entry.keywords:
            console.print(f"  • {kw}")
    if entry.eprint:
        console.print(f"  arXiv: {entry.eprint}")
    console.print(f"[dim]Run 'litbot db push' to share with your group.[/dim]")


# ── show ──────────────────────────────────────────────────────────────────────

@main.command("show")
@click.argument("key")
def show_cmd(key: str):
    """Print the BibTeX entry for a cite key."""
    library = _load_merged()
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
    library = _load_merged()
    entry = library.get(key)
    if entry is None:
        console.print(f"[red]{key} not found.[/red]")
        raise SystemExit(1)
    if not entry.eprint:
        console.print(f"[yellow]No arXiv ID for {key}.[/yellow]")
        raise SystemExit(1)
    status = "Opening cached PDF" if pdf.is_cached(key) else f"Fetching from arXiv:{entry.eprint}"
    console.print(f"{status}…")
    if not pdf.open_pdf(key, eprint=entry.eprint):
        console.print(f"[red]Failed to fetch PDF.[/red]")
        raise SystemExit(1)


# ── list ──────────────────────────────────────────────────────────────────────

@main.command("list")
@click.option("--keyword", "-k", default="", help="Filter by UAT keyword label.")
def list_cmd(keyword: str):
    """List papers in the library."""
    library = _load_merged()
    if keyword:
        config = get_config()
        from .uat import get_uat
        uat = get_uat(UAT_CACHE, auto_fetch=False)
        desc = uat.descendant_labels(keyword) if uat else {keyword}
        entries = library.by_keyword(keyword, descendant_labels=desc)
    else:
        entries = library.entries()
    entries = sorted(entries, key=lambda e: e.year, reverse=True)
    _print_entry_table(entries)
    console.print(f"\n{len(entries)} paper(s)")


# ── keywords ──────────────────────────────────────────────────────────────────

@main.command("keywords")
def keywords_cmd():
    """List all unique keywords across the library."""
    library = _load_merged()
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

def _get_config():
    try:
        return get_config()
    except RuntimeError as e:
        console.print(f"[red]{e}[/red]")
        raise SystemExit(1)


def _load_merged() -> MergedLibrary:
    config = _get_config()
    libs = [Library(root=p) for p in config.databases.values() if p.exists()]
    return MergedLibrary(libs)


def _search_local(query: str, limit: int):
    library = _load_merged()
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
        first_author = (a.author[0].split(",")[0] if a.author else "")
        title = (a.title[0] if a.title else "")
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
