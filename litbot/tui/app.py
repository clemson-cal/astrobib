from __future__ import annotations

import time
from pathlib import Path

from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.message import Message
from textual.screen import ModalScreen
from textual.widgets import (
    DataTable,
    Footer,
    Header,
    Input,
    Label,
    Static,
    TabbedContent,
    TabPane,
    Tabs,
)

from ..library import Entry, Library
from ..state import UAT_CACHE, get_library_path, get_token
from ..uat import UAT, Concept, get_uat
from . import tabs_state


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
        from rich.text import Text
        content = Text()
        content.append(entry.key, style="bold cyan")
        content.append("\n\n")
        content.append(entry.title, style="bold")
        content.append("\n\n")
        content.append(entry.author, style="dim")
        content.append("\n\n")
        content.append(entry.year, style="green")
        link_line = _link_line(adsurl=entry.adsurl, eprint=entry.eprint, doi=entry.doi)
        if len(link_line):
            content.append("\n")
            content.append_text(link_line)
        if entry.keywords:
            content.append("\n\n")
            content.append("Keywords:", style="yellow")
            for kw in entry.keywords:
                content.append(f"\n  • {kw}")
        abstract = entry.data.get("abstract", "")
        if abstract:
            content.append(f"\n\n{abstract[:600]}{'…' if len(abstract) > 600 else ''}", style="dim")
        self.update(content)

    def show_ads_article(self, article, entry: Entry | None = None) -> None:
        if article is None:
            self.update("")
            return
        from rich.text import Text
        title = article.title[0] if article.title else ""
        authors = article.author or []
        author_str = ", ".join(a.split(",")[0] for a in authors[:3])
        if len(authors) > 3:
            author_str += " et al."
        abstract = article.abstract or ""
        eprint = _arxiv_id_from_identifiers(article.identifier or [])
        doi = (article.doi or [""])[0]
        content = Text()
        content.append(article.bibcode, style="bold cyan")
        content.append("\n\n")
        content.append(title, style="bold")
        content.append("\n\n")
        content.append(author_str, style="dim")
        content.append("\n\n")
        content.append(str(article.year or ""), style="green")
        link_line = _link_line(
            adsurl=f"https://ui.adsabs.harvard.edu/abs/{article.bibcode}",
            eprint=entry.eprint if entry else eprint,
            doi=entry.doi if entry else doi,
        )
        if len(link_line):
            content.append("\n")
            content.append_text(link_line)
        if entry:
            if entry.keywords:
                content.append("\n\n")
                content.append("Keywords:", style="yellow")
                for kw in entry.keywords:
                    content.append(f"\n  • {kw}")
            content.append("\n\n✓ in library", style="dim")
        if abstract:
            content.append(f"\n\n{abstract[:600]}{'…' if len(abstract) > 600 else ''}", style="dim")
        self.update(content)

    def show_concept(self, concept: Concept | None, uat: UAT) -> None:
        if concept is None:
            self.update("[dim]Select a concept.[/dim]")
            return
        parents = uat.parents(concept.uid)
        breadcrumb = (
            " › ".join(p.label for p in parents)
            + (" › " if parents else "")
            + concept.label
        )
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


# ── Library view ──────────────────────────────────────────────────────────────

class LibraryView(Static):
    """Sortable, filterable paper list with multi-select."""

    DEFAULT_CSS = """
    LibraryView {
        height: 100%;
        layout: vertical;
    }
    LibraryView Input {
        height: 3;
        dock: top;
        display: none;
    }
    LibraryView Input.active {
        display: block;
    }
    LibraryView DataTable {
        height: 1fr;
    }
    """

    class EntryHighlighted(Message):
        def __init__(self, entry: Entry | None) -> None:
            super().__init__()
            self.entry = entry

    def __init__(self) -> None:
        super().__init__(id="library-view")
        self._all_entries: list[Entry] = []
        self._filtered: list[Entry] = []
        self._sort_col: str = "year"
        self._sort_reverse: bool = True
        self._selected_keys: set[str] = set()
        self._highlighted_entry: Entry | None = None

    def compose(self) -> ComposeResult:
        yield Input(placeholder="Filter by author, title, key, or keyword…", id="lib-filter")
        yield DataTable(id="lib-table", cursor_type="row")

    def on_mount(self) -> None:
        t = self.query_one("#lib-table", DataTable)
        t.add_column(" ", key="sel", width=2)
        t.add_column("↓", key="pdf", width=2)
        t.add_column("Year", key="year", width=6)
        t.add_column("Author", key="author", width=20)
        t.add_column("Title", key="title")
        t.add_column("Keywords", key="keywords", width=28)

    def load(self, library: Library | None) -> None:
        self._all_entries = library.entries() if library else []
        self._selected_keys.clear()
        filter_val = self.query_one("#lib-filter", Input).value
        self._apply_filter(filter_val)

    def focus_filter(self) -> None:
        inp = self.query_one("#lib-filter", Input)
        inp.add_class("active")
        inp.focus()

    def clear_filter(self) -> None:
        inp = self.query_one("#lib-filter", Input)
        inp.value = ""
        inp.remove_class("active")
        self.query_one("#lib-table", DataTable).focus()

    def toggle_selection(self) -> None:
        if self._highlighted_entry is None:
            return
        key = self._highlighted_entry.key
        if key in self._selected_keys:
            self._selected_keys.discard(key)
            new_val = ""
        else:
            self._selected_keys.add(key)
            new_val = "✓"
        try:
            self.query_one("#lib-table", DataTable).update_cell(key, "sel", new_val)
        except Exception:
            self._refresh_table()

    def get_selected_keys(self) -> list[str]:
        return list(self._selected_keys)

    def _apply_filter(self, text: str) -> None:
        q = text.lower().strip()
        if q:
            self._filtered = [
                e for e in self._all_entries
                if q in e.title.lower()
                or q in e.author.lower()
                or q in e.key.lower()
                or any(q in kw.lower() for kw in e.keywords)
            ]
        else:
            self._filtered = list(self._all_entries)
        self._refresh_table()

    def _sort_key(self, e: Entry) -> str:
        if self._sort_col == "year":
            return e.year
        if self._sort_col == "author":
            return e.first_author_last.lower()
        if self._sort_col == "title":
            return e.title.lower()
        return e.year

    def _refresh_table(self) -> None:
        from .. import pdf as _pdf
        t = self.query_one("#lib-table", DataTable)
        t.clear()
        entries = sorted(self._filtered, key=self._sort_key, reverse=self._sort_reverse)
        for e in entries:
            sel = "✓" if e.key in self._selected_keys else ""
            cached = "↓" if _pdf.is_cached(e.key) else ""
            kws = ", ".join(e.keywords[:3])
            if len(e.keywords) > 3:
                kws += "…"
            title = e.title[:52] + "…" if len(e.title) > 52 else e.title
            t.add_row(sel, cached, e.year, e.first_author_last, title, kws, key=e.key)

    def refresh_pdf_status(self) -> None:
        from .. import pdf as _pdf
        t = self.query_one("#lib-table", DataTable)
        for e in self._all_entries:
            try:
                t.update_cell(e.key, "pdf", "↓" if _pdf.is_cached(e.key) else "")
            except Exception:
                pass

    @on(Input.Changed, "#lib-filter")
    def _on_filter_changed(self, event: Input.Changed) -> None:
        self._apply_filter(event.value)
        if not event.value:
            event.input.remove_class("active")
            self.query_one("#lib-table", DataTable).focus()

    @on(DataTable.RowHighlighted, "#lib-table")
    @on(DataTable.RowSelected, "#lib-table")
    def _on_row_event(self, event) -> None:
        key = event.row_key.value if event.row_key else None
        if not key:
            return
        entry = next((e for e in self._all_entries if e.key == key), None)
        self._highlighted_entry = entry
        self.post_message(self.EntryHighlighted(entry))

    @on(DataTable.HeaderSelected, "#lib-table")
    def _on_header_selected(self, event: DataTable.HeaderSelected) -> None:
        try:
            col = event.column_key.value
        except AttributeError:
            return
        if col not in ("year", "author", "title"):
            return
        if self._sort_col == col:
            self._sort_reverse = not self._sort_reverse
        else:
            self._sort_col = col
            self._sort_reverse = (col == "year")
        self._refresh_table()


# ── ADS results view ──────────────────────────────────────────────────────────

class AdsView(Static):
    """ADS search results for one query tab."""

    DEFAULT_CSS = """
    AdsView {
        height: 100%;
    }
    AdsView DataTable {
        height: 100%;
    }
    """

    class ArticleHighlighted(Message):
        def __init__(self, article) -> None:
            super().__init__()
            self.article = article

    def __init__(self, query: str, tab_id: str) -> None:
        super().__init__(id=f"ads-view-{tab_id}")
        self.query = query
        self.tab_id = tab_id
        self._articles: list = []
        self._selected_article = None

    def compose(self) -> ComposeResult:
        yield DataTable(cursor_type="row")

    def on_mount(self) -> None:
        t = self.query_one(DataTable)
        t.add_column("✓", key="indb", width=3)
        t.add_column("Year", key="year", width=6)
        t.add_column("Author", key="author", width=20)
        t.add_column("Title", key="title")

    def load_articles(self, articles: list, library: Library | None) -> None:
        self._articles = articles
        t = self.query_one(DataTable)
        t.clear()
        for a in articles:
            in_db = "✓" if library and library.has_bibcode(a.bibcode) else ""
            first_author = a.author[0].split(",")[0] if a.author else ""
            title = a.title[0] if a.title else ""
            short_title = title[:55] + "…" if len(title) > 55 else title
            t.add_row(in_db, str(a.year or ""), first_author, short_title, key=a.bibcode)

    def update_in_db(self, library: Library | None) -> None:
        t = self.query_one(DataTable)
        for a in self._articles:
            in_db = "✓" if library and library.has_bibcode(a.bibcode) else ""
            try:
                t.update_cell(a.bibcode, "indb", in_db)
            except Exception:
                pass

    @on(DataTable.RowHighlighted)
    @on(DataTable.RowSelected)
    def _on_row_event(self, event) -> None:
        key = event.row_key.value if event.row_key else None
        if not key:
            return
        article = next((a for a in self._articles if a.bibcode == key), None)
        self._selected_article = article
        self.post_message(self.ArticleHighlighted(article))


# ── Modals ────────────────────────────────────────────────────────────────────

class AdsSearchModal(ModalScreen[str]):
    DEFAULT_CSS = """
    AdsSearchModal { align: center middle; }
    AdsSearchModal Vertical {
        width: 64; height: auto;
        border: round $accent; padding: 1 2; background: $surface;
    }
    """

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("New ADS search  [dim](Enter / Escape)[/dim]")
            yield Input(placeholder="author, title, or topic…", id="ads-input")

    def on_mount(self) -> None:
        self.query_one(Input).focus()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        self.dismiss(event.value.strip())

    def on_key(self, event) -> None:
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

    def on_mount(self) -> None:
        self.query_one("#bibcode-input", Input).focus()

    def on_key(self, event) -> None:
        if event.key == "escape":
            self.dismiss(None)
        elif event.key == "enter":
            bibcode = self.query_one("#bibcode-input", Input).value.strip()
            extra_kw = self.query_one("#kw-input", Input).value.strip()
            if bibcode:
                self.dismiss((bibcode, extra_kw))


class TokenModal(ModalScreen[str]):
    DEFAULT_CSS = """
    TokenModal { align: center middle; }
    TokenModal Vertical {
        width: 70; height: auto;
        border: round $warning; padding: 1 2; background: $surface;
    }
    """

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("ADS API token required  [dim](Escape to cancel)[/dim]")
            yield Label("[dim]Get one at: https://ui.adsabs.harvard.edu/user/settings/token[/dim]")
            yield Input(placeholder="Paste your ADS token here…", id="token-input", password=True)

    def on_mount(self) -> None:
        self.query_one(Input).focus()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        token = event.value.strip()
        if token:
            from ..state import set_token
            set_token(token)
            self.dismiss(token)

    def on_key(self, event) -> None:
        if event.key == "escape":
            self.dismiss("")


# ── Main app ──────────────────────────────────────────────────────────────────

class LitbotApp(App):
    TITLE = "litbot"
    CSS = """
    Screen { layout: vertical; }
    #body { layout: horizontal; height: 1fr; }
    TabbedContent { width: 1fr; height: 100%; }
    TabbedContent ContentSwitcher { height: 1fr; }
    TabPane { padding: 0; height: 100%; }
    #detail { width: 42; min-width: 30; }
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
        Binding("d", "remove_paper", "Remove"),
        Binding("o", "open_pdf", "Open PDF"),
        Binding("X", "clear_pdf", "Clear PDF"),
        Binding("/", "filter", "Filter"),
        Binding("S", "ads_search", "ADS search"),
        Binding("r", "refresh_tab", "Refresh"),
        Binding("ctrl+w", "close_tab", "Close tab", show=False),
        Binding("[", "prev_tab", "Prev tab", show=False),
        Binding("]", "next_tab", "Next tab", show=False),
        Binding("space", "toggle_select", "Select", show=False),
        Binding("e", "export_selected", "Export"),
        Binding("T", "set_token", "Token", show=False),
        Binding("u", "uat_browser", "UAT"),
        Binding("question_mark", "help", "Help"),
        Binding("escape", "clear_filter", "Clear", show=False),
    ]

    def __init__(self) -> None:
        super().__init__()
        self._library: Library | None = None
        self._uat: UAT | None = None
        self._tab_states: list[dict] = []
        self._ads_views: dict[str, AdsView] = {}

    def compose(self) -> ComposeResult:
        yield Header()
        with Horizontal(id="body"):
            with TabbedContent(id="tabs"):
                with TabPane("Library", id="pane-library"):
                    yield LibraryView()
            yield DetailPanel(id="detail")
        yield Static("", id="status-bar")
        yield Footer()

    async def on_mount(self) -> None:
        self._uat = get_uat(UAT_CACHE, auto_fetch=False)

        self._library = Library(root=get_library_path())
        self.query_one(LibraryView).load(self._library)

        self._tab_states = tabs_state.load()
        has_token = bool(get_token())
        for tab_data in self._tab_states:
            await self._create_ads_tab(tab_data, fetch=has_token)

        n = len(self._library.entries())
        uat_note = f" · UAT ({len(self._uat)})" if self._uat else ""
        token_note = "" if get_token() else "  [yellow]T: set ADS token[/yellow]"
        self._set_status(f"{n} papers{uat_note}{token_note}")

    # ── Tab helpers ───────────────────────────────────────────────────────────

    def _active_pane_id(self) -> str:
        return self.query_one(TabbedContent).active or ""

    def _active_ads_view(self) -> AdsView | None:
        pane_id = self._active_pane_id()
        if not pane_id or pane_id == "pane-library":
            return None
        tab_id = pane_id.removeprefix("pane-")
        return self._ads_views.get(tab_id)

    async def _create_ads_tab(self, tab_data: dict, fetch: bool = True) -> AdsView:
        tab_id = tab_data["id"]
        ads_view = AdsView(query=tab_data["query"], tab_id=tab_id)
        self._ads_views[tab_id] = ads_view
        pane = TabPane(tab_data["label"], ads_view, id=f"pane-{tab_id}")
        await self.query_one(TabbedContent).add_pane(pane)
        if fetch:
            await self._do_refresh(ads_view, tab_data)
        return ads_view

    async def _do_refresh(self, ads_view: AdsView, tab_data: dict) -> None:
        self._set_status(f"Searching ADS for '{ads_view.query}'…")
        try:
            from .. import ads_client
            articles = ads_client.search(ads_view.query, limit=20)
            ads_view.load_articles(articles, self._library)
            tab_data["bibcodes"] = [a.bibcode for a in articles]
            tab_data["refreshed"] = int(time.time())
            tabs_state.save(self._tab_states)
            ads_client.refresh_quota()
            n = len(articles)
            self._set_status(
                f"[yellow]{n} result(s) for '{ads_view.query}'[/yellow]"
                f"  [dim]a: add · d: remove · r: refresh · Ctrl+W: close{_quota_str()}[/dim]"
            )
        except RuntimeError as e:
            self._set_status(f"[red]{e}[/red]")

    def _reload_library(self) -> None:
        self._library = Library(root=get_library_path())
        self.query_one(LibraryView).load(self._library)
        for av in self._ads_views.values():
            av.update_in_db(self._library)

    # ── Tab events ────────────────────────────────────────────────────────────

    @on(TabbedContent.TabActivated)
    def _on_tab_activated(self, event: TabbedContent.TabActivated) -> None:
        self.query_one(DetailPanel).show_entry(None)
        self.refresh_bindings()
        pane_id = self._active_pane_id()
        if pane_id == "pane-library":
            lib_view = self.query_one(LibraryView)
            n = len(lib_view._filtered)
            total = len(lib_view._all_entries)
            shown = f" · {n} shown" if n != total else ""
            self._set_status(f"{total} papers{shown}  [dim]/ filter · Space select · e export[/dim]")
        elif pane_id:
            tab_id = pane_id.removeprefix("pane-")
            ads_view = self._ads_views.get(tab_id)
            if ads_view:
                if not ads_view._articles:
                    self._set_status(
                        f"[dim]'{ads_view.query}' — press [bold]r[/bold] to load results[/dim]"
                    )
                else:
                    n = len(ads_view._articles)
                    self._set_status(
                        f"[yellow]{n} result(s) for '{ads_view.query}'[/yellow]"
                        f"  [dim]r: refresh · Ctrl+W: close{_quota_str()}[/dim]"
                    )

    # ── Message handlers ──────────────────────────────────────────────────────

    @on(LibraryView.EntryHighlighted)
    def _on_entry_highlighted(self, event: LibraryView.EntryHighlighted) -> None:
        if self._active_pane_id() == "pane-library":
            self.query_one(DetailPanel).show_entry(event.entry)
            self.refresh_bindings()

    @on(AdsView.ArticleHighlighted)
    def _on_article_highlighted(self, event: AdsView.ArticleHighlighted) -> None:
        if event.article is None:
            return
        entry = self._library.get_by_bibcode(event.article.bibcode) if self._library else None
        self.query_one(DetailPanel).show_ads_article(event.article, entry=entry)
        self.refresh_bindings()

    # ── Actions ───────────────────────────────────────────────────────────────

    def action_next_tab(self) -> None:
        self._switch_tab(+1)

    def action_prev_tab(self) -> None:
        self._switch_tab(-1)

    def _switch_tab(self, direction: int) -> None:
        tabs = self.query_one(Tabs)
        if direction > 0:
            tabs.action_next_tab()
        else:
            tabs.action_previous_tab()
        self.call_after_refresh(self._focus_active_table)

    def _focus_active_table(self) -> None:
        pane_id = self._active_pane_id()
        if pane_id == "pane-library":
            try:
                self.query_one(LibraryView).query_one(DataTable).focus()
            except Exception:
                pass
        elif pane_id:
            tab_id = pane_id.removeprefix("pane-")
            ads_view = self._ads_views.get(tab_id)
            if ads_view:
                try:
                    ads_view.query_one(DataTable).focus()
                except Exception:
                    pass

    def action_filter(self) -> None:
        if self._active_pane_id() == "pane-library":
            self.query_one(LibraryView).focus_filter()

    def action_clear_filter(self) -> None:
        if self._active_pane_id() == "pane-library":
            self.query_one(LibraryView).clear_filter()

    def action_toggle_select(self) -> None:
        if self._active_pane_id() != "pane-library":
            return
        focused = self.focused
        if isinstance(focused, Input):
            return
        self.query_one(LibraryView).toggle_selection()

    def action_export_selected(self) -> None:
        if self._active_pane_id() != "pane-library":
            return
        lib_view = self.query_one(LibraryView)
        keys = lib_view.get_selected_keys()
        if not keys:
            self._set_status("[yellow]No papers selected — use Space to select rows.[/yellow]")
            return
        output = Path.cwd() / "litbot-export.bib"
        blocks = []
        for key in keys:
            entry = self._library.get(key) if self._library else None
            if entry:
                blocks.append(entry.path.read_text())
        output.write_text("\n".join(blocks))
        self._set_status(f"[green]Exported {len(blocks)} paper(s) → {output}[/green]")

    def action_ads_search(self) -> None:
        if not get_token():
            self.push_screen(TokenModal(), self._after_token_set)
            return

        async def handle(query: str) -> None:
            if not query:
                return
            tab_data = tabs_state.make_tab(query)
            self._tab_states.append(tab_data)
            await self._create_ads_tab(tab_data, fetch=True)

        self.push_screen(AdsSearchModal(), handle)

    def _after_token_set(self, token: str) -> None:
        if token:
            self._set_status("[green]ADS token saved. Press S to search.[/green]")

    def action_set_token(self) -> None:
        self.push_screen(TokenModal(), self._after_token_set)

    async def action_close_tab(self) -> None:
        pane_id = self._active_pane_id()
        if not pane_id or pane_id == "pane-library":
            return
        tab_id = pane_id.removeprefix("pane-")
        self._ads_views.pop(tab_id, None)
        self._tab_states = [t for t in self._tab_states if t["id"] != tab_id]
        tabs_state.save(self._tab_states)
        await self.query_one(TabbedContent).remove_pane(pane_id)

    async def action_refresh_tab(self) -> None:
        ads_view = self._active_ads_view()
        if ads_view is None:
            return
        tab_data = next((t for t in self._tab_states if t["id"] == ads_view.tab_id), None)
        if tab_data:
            await self._do_refresh(ads_view, tab_data)

    def action_add_paper(self) -> None:
        ads_view = self._active_ads_view()
        if ads_view is not None:
            article = ads_view._selected_article
            if article is None:
                self._set_status("[yellow]Select an article first.[/yellow]")
                return
            if self._library and self._library.has_bibcode(article.bibcode):
                self._set_status("[yellow]Already in library.[/yellow]")
                return
            self._set_status(f"Fetching {article.bibcode}…")
            self.run_worker(self._fetch_and_add(article.bibcode, ads_view), exclusive=True)
        else:
            async def handle(result: tuple[str, str] | None) -> None:
                if result is None:
                    return
                bibcode, extra_kw = result
                self._set_status(f"Fetching {bibcode}…")
                try:
                    from .. import ads_client
                    data = ads_client.fetch_bibtex(bibcode)
                    if data is None:
                        self._set_status(f"[red]Could not fetch {bibcode}[/red]")
                        return
                    if extra_kw:
                        existing_kw = data.get("keywords", "")
                        data["keywords"] = ", ".join(filter(None, [existing_kw, extra_kw]))
                    entry = self._library.save_entry(data)
                    self._reload_library()
                    self._set_status(f"[green]Added {entry.key}[/green]")
                except Exception as exc:
                    self._set_status(f"[red]{exc}[/red]")
            self.push_screen(AddModal(), handle)

    async def _fetch_and_add(self, bibcode: str, ads_view: AdsView) -> None:
        try:
            from .. import ads_client
            data = ads_client.fetch_bibtex(bibcode)
            if data is None:
                self._set_status(f"[red]Could not fetch {bibcode}[/red]")
                return
            entry = self._library.save_entry(data)
            self._reload_library()
            ads_view.update_in_db(self._library)
            self.query_one(DetailPanel).show_entry(self._library.get(entry.key))
            self.refresh_bindings()
            ads_client.refresh_quota()
            self._set_status(f"[green]Added {entry.key}[/green]{_quota_str()}")
        except Exception as exc:
            self._set_status(f"[red]{exc}[/red]")

    def action_remove_paper(self) -> None:
        ads_view = self._active_ads_view()
        if ads_view is None:
            if self._active_pane_id() == "pane-library":
                entry = self.query_one(LibraryView)._highlighted_entry
                if entry and self._library:
                    self._library.remove_entry(entry.key)
                    self._reload_library()
                    self.refresh_bindings()
                    self._set_status(f"[green]Removed {entry.key}[/green]")
            return
        article = ads_view._selected_article
        if article is None:
            self._set_status("[yellow]Select an article first.[/yellow]")
            return
        if self._library is None:
            return
        entry = self._library.get_by_bibcode(article.bibcode)
        if entry is None:
            self._set_status("[yellow]Not in library.[/yellow]")
            return
        self._library.remove_entry(entry.key)
        self._reload_library()
        ads_view.update_in_db(self._library)
        self.refresh_bindings()
        self._set_status(f"[green]Removed {entry.key}[/green]")

    def action_clear_pdf(self) -> None:
        from .. import pdf as _pdf
        ads_view = self._active_ads_view()
        if ads_view is not None:
            article = ads_view._selected_article
            if article:
                path = _pdf.cache_path(article.bibcode)
                if path.exists():
                    path.unlink()
                    self.refresh_bindings()
                    self._set_status(f"[green]Cleared cached PDF for {article.bibcode}[/green]")
        elif self._active_pane_id() == "pane-library":
            entry = self.query_one(LibraryView)._highlighted_entry
            if entry:
                path = _pdf.cache_path(entry.key)
                if path.exists():
                    path.unlink()
                    self.query_one(LibraryView).refresh_pdf_status()
                    self.refresh_bindings()
                    self._set_status(f"[green]Cleared cached PDF for {entry.key}[/green]")

    def action_open_pdf(self) -> None:
        from .. import pdf, ads_client as _ac
        ads_view = self._active_ads_view()
        if ads_view is not None:
            article = ads_view._selected_article
            if article is None:
                self._set_status("[yellow]Select an article first.[/yellow]")
                return
            eprint = _ac.arxiv_id_from_article(article)
            if not eprint:
                self._set_status(f"[yellow]No arXiv ID for {article.bibcode}[/yellow]")
                return
            key = article.bibcode
        else:
            lib_view = self.query_one(LibraryView)
            entry = lib_view._highlighted_entry
            if entry is None:
                self._set_status("[yellow]Select a paper first.[/yellow]")
                return
            if not entry.eprint:
                self._set_status(f"[yellow]No arXiv ID for {entry.key}[/yellow]")
                return
            eprint = entry.eprint
            key = entry.key
        cached = pdf.is_cached(key)
        self._set_status(
            f"Opening {key}…" if cached else f"Fetching {key} from arXiv:{eprint}…"
        )
        if not pdf.open_pdf(key, eprint=eprint):
            self._set_status(f"[red]Failed to fetch PDF for {key}[/red]")
        else:
            self.query_one(LibraryView).refresh_pdf_status()
            self.refresh_bindings()

    def action_uat_browser(self) -> None:
        if self._uat is None:
            self._set_status("[yellow]UAT not cached — run: litbot uat update[/yellow]")
            return
        from .uat_browser import UATBrowserScreen
        self.push_screen(UATBrowserScreen(self._uat))

    def action_help(self) -> None:
        from .help_screen import HelpScreen
        self.push_screen(HelpScreen())

    def check_action(self, action: str, parameters: tuple[object, ...]) -> bool | None:
        from .. import pdf as _pdf
        pane_id = self._active_pane_id()
        if action == "clear_pdf":
            if pane_id == "pane-library":
                entry = self.query_one(LibraryView)._highlighted_entry
                return entry is not None and _pdf.is_cached(entry.key)
            ads_view = self._active_ads_view()
            if ads_view and ads_view._selected_article:
                return _pdf.is_cached(ads_view._selected_article.bibcode)
            return False
        if action == "add_paper":
            if pane_id == "pane-library":
                lib_view = self.query_one(LibraryView)
                return lib_view._highlighted_entry is None
            ads_view = self._active_ads_view()
            if ads_view is not None:
                article = ads_view._selected_article
                if article and self._library:
                    return self._library.get_by_bibcode(article.bibcode) is None
            return True
        if action == "remove_paper":
            if pane_id == "pane-library":
                lib_view = self.query_one(LibraryView)
                return lib_view._highlighted_entry is not None
            ads_view = self._active_ads_view()
            if ads_view is not None:
                article = ads_view._selected_article
                if article and self._library:
                    return self._library.get_by_bibcode(article.bibcode) is not None
            return False
        return True

    def _set_status(self, msg: str) -> None:
        self.query_one("#status-bar", Static).update(msg)


# ── Helpers ───────────────────────────────────────────────────────────────────

def _link_line(adsurl: str, eprint: str, doi: str) -> "Text":
    from rich.text import Text
    from rich.style import Style
    result = Text()
    parts = []
    if adsurl:
        parts.append(Text("ADS", style=Style(color="cyan", link=adsurl)))
    if eprint:
        parts.append(Text(f"arXiv:{eprint}", style=Style(color="cyan", link=f"https://arxiv.org/abs/{eprint}")))
    if doi:
        parts.append(Text("DOI", style=Style(color="cyan", link=f"https://doi.org/{doi}")))
    for i, part in enumerate(parts):
        if i:
            result.append("  ")
        result.append_text(part)
    return result


def _arxiv_id_from_identifiers(identifiers: list[str]) -> str:
    for ident in identifiers:
        if ident.startswith("arXiv:"):
            return ident[6:]
    return ""


def _quota_str() -> str:
    from .. import ads_client
    quota = ads_client.get_quota()
    if not quota or not quota.get("limit"):
        return ""
    remaining = quota["remaining"]
    limit = quota["limit"]
    color = "green" if remaining > limit * 0.2 else "yellow" if remaining > 0 else "red"
    return f" · ADS [{color}]{remaining}/{limit}[/{color}]"
