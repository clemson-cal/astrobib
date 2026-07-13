from __future__ import annotations

from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import (
    DataTable,
    Footer,
    Header,
    Input,
    Label,
    Static,
    Tree,
)
from ..config import get_config, UAT_CACHE
from ..library import Entry, Library, MergedLibrary
from ..uat import UAT, Concept, get_uat


# ── Detail panel ──────────────────────────────────────────────────────────────

class DetailPanel(Static):
    DEFAULT_CSS = """
    DetailPanel {
        height: 100%;
        padding: 1 2;
        border-left: solid $panel-lighten-1;
        overflow-y: auto;
    }
    """

    def show_entry(self, entry: Entry | None) -> None:
        if entry is None:
            self.update("")
            return
        lines = [
            f"[bold cyan]{entry.key}[/bold cyan]\n",
            f"[bold]{entry.title}[/bold]\n",
            f"[dim]{entry.author}[/dim]\n",
            f"[green]{entry.year}[/green]",
        ]
        if entry.eprint:
            lines.append(f"  arXiv:[cyan]{entry.eprint}[/cyan]")
        if entry.doi:
            lines.append(f"  DOI: {entry.doi}")
        if entry.keywords:
            lines.append("\n[yellow]Keywords:[/yellow]")
            for kw in entry.keywords:
                lines.append(f"  • {kw}")
        abstract = entry.data.get("abstract", "")
        if abstract:
            lines.append(
                f"\n[dim]{abstract[:600]}{'…' if len(abstract) > 600 else ''}[/dim]"
            )
        self.update("\n".join(lines))

    def show_ads_article(self, article) -> None:
        if article is None:
            self.update("")
            return
        title = article.title[0] if article.title else ""
        authors = article.author or []
        author_str = ", ".join(a.split(",")[0] for a in authors[:3])
        if len(authors) > 3:
            author_str += " et al."
        abstract = article.abstract or ""
        lines = [
            f"[bold cyan]{article.bibcode}[/bold cyan]\n",
            f"[bold]{title}[/bold]\n",
            f"[dim]{author_str}[/dim]\n",
            f"[green]{article.year}[/green]",
        ]
        if abstract:
            lines.append(
                f"\n[dim]{abstract[:600]}{'…' if len(abstract) > 600 else ''}[/dim]"
            )
        lines.append("\n[dim]Press [bold]a[/bold] to add to library[/dim]")
        self.update("\n".join(lines))

    def show_concept(self, concept: Concept | None, uat: UAT) -> None:
        if concept is None:
            self.update("[dim]Select a concept.[/dim]")
            return
        parents = uat.parents(concept.uid)
        breadcrumb = " › ".join(p.label for p in parents) + (" › " if parents else "") + concept.label
        lines = [
            f"[dim]{breadcrumb}[/dim]\n",
            f"[bold cyan]{concept.label}[/bold cyan]  [dim]UID {concept.uid}[/dim]",
        ]
        if concept.alt_labels:
            lines.append("\n[yellow]Also known as:[/yellow]")
            for alt in concept.alt_labels:
                lines.append(f"  {alt}")
        if concept.definition:
            lines.append(f"\n[dim]{concept.definition}[/dim]")
        children = uat.children(concept.uid)
        if children:
            lines.append(f"\n[yellow]Narrower ({len(children)}):[/yellow]")
            for child in children[:20]:
                lines.append(f"  • {child.label}")
            if len(children) > 20:
                lines.append(f"  [dim]… and {len(children) - 20} more[/dim]")
        self.update("\n".join(lines))


# ── Modals ────────────────────────────────────────────────────────────────────

class SearchModal(ModalScreen[str]):
    DEFAULT_CSS = """
    SearchModal { align: center middle; }
    SearchModal Vertical {
        width: 60; height: auto;
        border: round $primary; padding: 1 2; background: $surface;
    }
    """

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Search library  [dim](Enter / Escape)[/dim]")
            yield Input(placeholder="author, title, or cite key…", id="search-input")

    def on_mount(self):
        self.query_one(Input).focus()

    def on_input_submitted(self, event: Input.Submitted):
        self.dismiss(event.value)

    def on_key(self, event):
        if event.key == "escape":
            self.dismiss("")


class AdsSearchModal(ModalScreen[str]):
    DEFAULT_CSS = """
    AdsSearchModal { align: center middle; }
    AdsSearchModal Vertical {
        width: 60; height: auto;
        border: round $accent; padding: 1 2; background: $surface;
    }
    """

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Search ADS  [dim](Enter / Escape)[/dim]")
            yield Input(placeholder="author, title, or topic…", id="ads-input")

    def on_mount(self):
        self.query_one(Input).focus()

    def on_input_submitted(self, event: Input.Submitted):
        self.dismiss(event.value)

    def on_key(self, event):
        if event.key == "escape":
            self.dismiss("")


class AddModal(ModalScreen[tuple[str, str] | None]):
    DEFAULT_CSS = """
    AddModal { align: center middle; }
    AddModal Vertical {
        width: 70; height: auto;
        border: round $primary; padding: 1 2; background: $surface;
    }
    """

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Add paper from ADS  [dim](Escape to cancel)[/dim]")
            yield Label("ADS bibcode:")
            yield Input(placeholder="e.g. 2020ApJ...900...12S", id="bibcode-input")
            yield Label("Extra keywords  [dim](optional, comma-separated UAT labels)[/dim]:")
            yield Input(placeholder="e.g. Magnetohydrodynamical simulations", id="kw-input")

    def on_mount(self):
        self.query_one("#bibcode-input", Input).focus()

    def on_key(self, event):
        if event.key == "escape":
            self.dismiss(None)
        elif event.key == "enter":
            bibcode = self.query_one("#bibcode-input", Input).value.strip()
            extra_kw = self.query_one("#kw-input", Input).value.strip()
            if bibcode:
                self.dismiss((bibcode, extra_kw))


# ── Main app ──────────────────────────────────────────────────────────────────

class LitbotApp(App):
    TITLE = "litbot"
    CSS = """
    Screen { layout: vertical; }
    #body { layout: horizontal; height: 1fr; }
    #left-panel {
        width: 28;
        min-width: 22;
        border-right: solid $panel-lighten-1;
    }
    #keyword-tree { width: 100%; height: 100%; }
    #uat-tree     { width: 100%; height: 100%; display: none; }
    #paper-table  { width: 1fr; }
    #detail       { width: 40; min-width: 30; }
    #status-bar {
        height: 1;
        background: $panel;
        padding: 0 1;
        color: $text-muted;
    }
    """

    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("a", "add_paper", "Add"),
        Binding("o", "open_pdf", "Open PDF"),
        Binding("/", "search", "Search"),
        Binding("S", "ads_search", "ADS search"),
        Binding("u", "toggle_uat", "UAT"),
        Binding("question_mark", "help", "Help"),
        Binding("escape", "show_all", "All", show=False),
        Binding("t", "focus_left", "Left panel", show=False),
    ]

    def __init__(self):
        super().__init__()
        self._library: MergedLibrary | None = None
        self._uat: UAT | None = None
        self._current_entries: list[Entry] = []
        self._selected_entry: Entry | None = None
        self._left_mode: str = "library"   # "library" | "uat"
        self._uat_tree_built: bool = False
        self._ads_results: dict = {}       # bibcode -> ADS Article
        self._selected_ads = None
        self._view_mode: str = "library"   # "library" | "ads"

    def compose(self) -> ComposeResult:
        yield Header()
        with Horizontal(id="body"):
            with Vertical(id="left-panel"):
                yield Tree("Keywords", id="keyword-tree")
                yield Tree("UAT", id="uat-tree")
            yield DataTable(id="paper-table", cursor_type="row")
            yield DetailPanel(id="detail")
        yield Static("", id="status-bar")
        yield Footer()

    def on_mount(self):
        self._uat = get_uat(UAT_CACHE, auto_fetch=False)

        try:
            config = get_config()
        except RuntimeError as e:
            self._set_status(f"[red]{e}[/red]  [dim]· u: UAT browser · ?: help[/dim]")
            self._setup_table()
            return

        libs = [Library(root=p) for p in config.databases.values() if p.exists()]
        self._library = MergedLibrary(libs)

        self._setup_table()
        self._build_keyword_tree()
        self._load_entries(self._library.entries())
        n = len(self._library.entries())
        db_note = f"{len(libs)} db"
        uat_note = f" · UAT ({len(self._uat)})" if self._uat else " · [dim]litbot uat update[/dim]"
        self._set_status(f"{n} papers · {db_note}{uat_note}  [dim]u: toggle UAT browser[/dim]")

    # ── Left panel: keyword tree ───────────────────────────────────────────────

    def _build_keyword_tree(self):
        tree = self.query_one("#keyword-tree", Tree)
        tree.root.expand()
        tree.root.add("All papers", data=("all", set()))

        if self._library is None:
            return

        all_kws = self._library.all_keywords()
        if not all_kws:
            return

        # Group keywords by their top-level UAT ancestor
        groups: dict[str, list[str]] = {}
        unclassified: list[str] = []

        for kw in all_kws:
            if self._uat:
                concept = self._uat.by_label(kw)
                if concept:
                    top = _top_ancestor(self._uat, concept.uid)
                    groups.setdefault(top, []).append(kw)
                    continue
            unclassified.append(kw)

        for group_label in sorted(groups):
            kws = groups[group_label]
            group_desc = self._uat.descendant_labels(group_label) if self._uat else {group_label}
            if len(kws) == 1 and kws[0].lower() == group_label.lower():
                tree.root.add(group_label, data=("uat", group_desc))
            else:
                group_node = tree.root.add(group_label, data=("uat", group_desc))
                for kw in sorted(kws):
                    kw_desc = self._uat.descendant_labels(kw) if self._uat else {kw}
                    group_node.add(kw, data=("uat", kw_desc))

        if unclassified:
            other_node = tree.root.add("[dim]other[/dim]", data=None)
            for kw in unclassified:
                other_node.add(kw, data=("kw", {kw}))

    @on(Tree.NodeSelected, "#keyword-tree")
    def on_keyword_selected(self, event: Tree.NodeSelected):
        if self._library is None or event.node.data is None:
            return
        self._view_mode = "library"
        kind, labels = event.node.data
        if kind == "all":
            entries = self._library.entries()
            self._load_entries(entries)
            self._set_status(f"{len(entries)} papers in library")
        else:
            entries = self._library.by_keyword("", descendant_labels=labels)
            label_str = next(iter(labels)) if len(labels) == 1 else event.node.label.plain
            self._load_entries(entries)
            self._set_status(f"{len(entries)} paper(s) tagged [{label_str}]")
        self.query_one("#detail", DetailPanel).show_entry(None)

    # ── Left panel: UAT tree ───────────────────────────────────────────────────

    def _build_uat_tree(self):
        if self._uat_tree_built or self._uat is None:
            return
        tree = self.query_one("#uat-tree", Tree)
        tree.root.expand()
        for concept in sorted(self._uat.top_level(), key=lambda c: c.label):
            tree.root.add(concept.label, data=concept, allow_expand=bool(concept.narrower))
        self._uat_tree_built = True

    @on(Tree.NodeExpanded, "#uat-tree")
    def on_uat_node_expanded(self, event: Tree.NodeExpanded):
        node = event.node
        concept: Concept | None = node.data
        if concept is None or node.children:
            return
        for child in sorted(self._uat.children(concept.uid), key=lambda c: c.label):
            node.add(child.label, data=child, allow_expand=bool(child.narrower))

    @on(Tree.NodeSelected, "#uat-tree")
    def on_uat_node_selected(self, event: Tree.NodeSelected):
        concept: Concept | None = event.node.data
        detail = self.query_one("#detail", DetailPanel)
        if concept is None or self._uat is None:
            detail.show_entry(None)
            return
        detail.show_concept(concept, self._uat)
        if self._library is not None:
            desc = self._uat.descendant_labels(concept.label)
            entries = self._library.by_keyword("", descendant_labels=desc)
            self._load_entries(entries)
            self._view_mode = "library"
            self._set_status(
                f"{len(entries)} paper(s) tagged [{concept.label}]  "
                f"[dim]· {len(desc)} UAT concepts in subtree[/dim]"
            )

    # ── Paper table ───────────────────────────────────────────────────────────

    def _setup_table(self):
        table = self.query_one("#paper-table", DataTable)
        table.add_columns("Cite key", "Year", "First author", "Title")

    def _load_entries(self, entries: list[Entry]):
        table = self.query_one("#paper-table", DataTable)
        table.clear()
        self._current_entries = sorted(entries, key=lambda e: e.year, reverse=True)
        for e in self._current_entries:
            title = e.title[:55] + "…" if len(e.title) > 55 else e.title
            table.add_row(e.key, e.year, e.first_author_last, title, key=e.key)
        self._selected_entry = None

    def _load_ads_articles(self, articles: list):
        table = self.query_one("#paper-table", DataTable)
        table.clear()
        self._current_entries = []
        self._selected_entry = None
        self._selected_ads = None
        for a in articles:
            first_author = a.author[0].split(",")[0] if a.author else ""
            title = a.title[0] if a.title else ""
            short_title = title[:55] + "…" if len(title) > 55 else title
            table.add_row(a.bibcode, str(a.year or ""), first_author, short_title, key=a.bibcode)

    @on(DataTable.RowHighlighted, "#paper-table")
    @on(DataTable.RowSelected, "#paper-table")
    def on_row_selected(self, event):
        row_key = event.row_key.value if event.row_key else None
        if not row_key:
            return
        detail = self.query_one("#detail", DetailPanel)
        if self._view_mode == "ads":
            self._selected_ads = self._ads_results.get(row_key)
            detail.show_ads_article(self._selected_ads)
        elif self._library:
            entry = self._library.get(row_key)
            self._selected_entry = entry
            detail.show_entry(entry)

    # ── Actions ───────────────────────────────────────────────────────────────

    def action_toggle_uat(self):
        if self._uat is None:
            self._set_status("[yellow]UAT not cached — run: litbot uat update[/yellow]")
            return
        self._left_mode = "uat" if self._left_mode == "library" else "library"
        in_uat = self._left_mode == "uat"
        self.query_one("#keyword-tree", Tree).display = not in_uat
        self.query_one("#uat-tree", Tree).display = in_uat
        if in_uat:
            self._build_uat_tree()
            self.query_one("#uat-tree", Tree).focus()
            self._set_status(f"UAT browser  [dim]· {len(self._uat)} concepts · u: back to library[/dim]")
        else:
            self.query_one("#keyword-tree", Tree).focus()
            n = len(self._library.entries()) if self._library else 0
            self._set_status(f"{n} papers  [dim]· u: UAT browser[/dim]")

    def action_search(self):
        async def handle(query: str):
            if not query or self._library is None:
                return
            q = query.lower()
            results = [
                e for e in self._library.entries()
                if q in e.title.lower()
                or q in e.author.lower()
                or q in e.key.lower()
                or any(q in kw.lower() for kw in e.keywords)
            ]
            self._view_mode = "library"
            self._load_entries(results)
            self._set_status(f'{len(results)} result(s) for "{query}"  [dim]· Escape: show all[/dim]')
        self.push_screen(SearchModal(), handle)

    def action_ads_search(self):
        async def handle(query: str):
            if not query:
                return
            self._set_status(f"Searching ADS for '{query}'…")
            try:
                from .. import ads_client
                articles = ads_client.search(query, limit=20)
            except RuntimeError as e:
                self._set_status(f"[red]{e}[/red]")
                return
            self._ads_results = {a.bibcode: a for a in articles}
            self._view_mode = "ads"
            self._load_ads_articles(articles)
            quota_str = _quota_str()
            self._set_status(
                f"[yellow]{len(articles)} ADS result(s) for '{query}'[/yellow]"
                f"  [dim]· a: add · Escape: back{quota_str}[/dim]"
            )
        self.push_screen(AdsSearchModal(), handle)

    def action_show_all(self):
        self._view_mode = "library"
        self._ads_results = {}
        self._selected_ads = None
        if self._library:
            entries = self._library.entries()
            self._load_entries(entries)
            self._set_status(f"{len(entries)} papers in library")

    def action_add_paper(self):
        if self._view_mode == "ads":
            if self._selected_ads is None:
                self._set_status("[yellow]Select an ADS result first.[/yellow]")
                return
            bibcode = self._selected_ads.bibcode
            self._set_status(f"Fetching {bibcode}…")
            self.run_worker(self._fetch_and_add(bibcode), exclusive=True)
            return

        async def handle(result: tuple[str, str] | None):
            if result is None:
                return
            bibcode, extra_kw = result
            self._set_status(f"Fetching {bibcode} from ADS…")
            try:
                from .. import ads_client
                data = ads_client.fetch_bibtex(bibcode)
                if data is None:
                    self._set_status(f"[red]Could not fetch {bibcode}[/red]")
                    return
                if extra_kw:
                    existing_kw = data.get("keywords", "")
                    data["keywords"] = ", ".join(filter(None, [existing_kw, extra_kw]))
                config = get_config()
                target_lib = Library(root=config.default_db_path)
                entry = target_lib.save_entry(data)
                from .. import db as dbmod
                try:
                    dbmod.commit_entry(config.default_db_path, entry.key)
                except Exception:
                    pass
                libs = [Library(root=p) for p in config.databases.values() if p.exists()]
                self._library = MergedLibrary(libs)
                self._load_entries(self._library.entries())
                self._set_status(f"[green]Added {entry.key} → '{config.default_database}'[/green]")
            except Exception as exc:
                self._set_status(f"[red]{exc}[/red]")
        self.push_screen(AddModal(), handle)

    async def _fetch_and_add(self, bibcode: str) -> None:
        try:
            from .. import ads_client
            data = ads_client.fetch_bibtex(bibcode)
            if data is None:
                self._set_status(f"[red]Could not fetch {bibcode}[/red]")
                return
            config = get_config()
            target_lib = Library(root=config.default_db_path)
            entry = target_lib.save_entry(data)
            from .. import db as dbmod
            try:
                dbmod.commit_entry(config.default_db_path, entry.key)
            except Exception:
                pass
            libs = [Library(root=p) for p in config.databases.values() if p.exists()]
            self._library = MergedLibrary(libs)
            quota_str = _quota_str()
            self._set_status(
                f"[green]Added {entry.key} → '{config.default_database}'[/green]"
                f"  [dim]· Escape: back to library{quota_str}[/dim]"
            )
        except Exception as exc:
            self._set_status(f"[red]{exc}[/red]")

    def action_open_pdf(self):
        entry = self._selected_entry
        if entry is None:
            self._set_status("Select a paper first.")
            return
        if not entry.eprint:
            self._set_status(f"[yellow]No arXiv ID for {entry.key}[/yellow]")
            return
        from .. import pdf
        cached = pdf.is_cached(entry.key)
        self._set_status(
            f"Opening {entry.key}…" if cached
            else f"Fetching {entry.key} from arXiv:{entry.eprint}…"
        )
        if not pdf.open_pdf(entry.key, eprint=entry.eprint):
            self._set_status(f"[red]Failed to fetch PDF for {entry.key}[/red]")

    def action_help(self):
        from .help_screen import HelpScreen
        self.push_screen(HelpScreen())

    def action_focus_left(self):
        tree_id = "#uat-tree" if self._left_mode == "uat" else "#keyword-tree"
        self.query_one(tree_id, Tree).focus()

    def _set_status(self, msg: str):
        self.query_one("#status-bar", Static).update(msg)


# ── Helpers ───────────────────────────────────────────────────────────────────

def _top_ancestor(uat: UAT, uid: str) -> str:
    """Walk up the UAT hierarchy and return the top-level concept's label."""
    concept = uat.by_uid(uid)
    while concept and concept.broader:
        parent = uat.by_uid(concept.broader[0])
        if parent is None:
            break
        concept = parent
    return concept.label if concept else ""


def _quota_str() -> str:
    """Return a short ADS quota string for the status bar, or empty string."""
    from .. import ads_client
    quota = ads_client.get_quota()
    if not quota or not quota.get("limit"):
        return ""
    remaining = quota["remaining"]
    limit = quota["limit"]
    color = "green" if remaining > limit * 0.2 else "yellow" if remaining > 0 else "red"
    return f" · ADS [{color}]{remaining}/{limit}[/{color}]"
