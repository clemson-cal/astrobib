from __future__ import annotations

import time
from pathlib import Path

from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.message import Message
from textual.screen import ModalScreen
from textual.widgets import (
    DataTable,
    Footer,
    Header,
    Input,
    Label,
    Link,
    Static,
    TabbedContent,
    TabPane,
    Tabs,
)

from ..library import Entry, Library
from ..state import (
    UAT_CACHE, PDF_CACHE_DIR, STATE_FILE,
    get_library_path, get_token, set_token, get_email, set_email,
)
from ..uat import UAT, Concept, get_uat
from . import tabs_state


# ── Detail panel ──────────────────────────────────────────────────────────────

class DetailPanel(VerticalScroll):
    DEFAULT_CSS = """
    DetailPanel {
        border-left: solid $panel-lighten-1;
    }
    DetailPanel #detail-body {
        height: auto;
        padding: 1 2 0 2;
    }
    DetailPanel #detail-links {
        height: auto;
        padding: 0 2 0 2;
        layout: horizontal;
    }
    DetailPanel #detail-footer {
        height: auto;
        padding: 0 2 1 2;
    }
    DetailPanel Link {
        width: auto;
        padding-right: 2;
        color: cyan;
        text-style: none;
        &:hover { text-style: underline; }
    }
    """

    def compose(self) -> ComposeResult:
        yield Static("", id="detail-body")
        with Horizontal(id="detail-links"):
            yield Link("ADS", url="", id="link-ads")
            yield Link("", url="", id="link-arxiv")
            yield Link("DOI", url="", id="link-doi")
        yield Static("", id="detail-footer")

    def _set_links(self, adsurl: str, eprint: str, doi: str) -> None:
        link_ads = self.query_one("#link-ads", Link)
        link_arxiv = self.query_one("#link-arxiv", Link)
        link_doi = self.query_one("#link-doi", Link)

        link_ads.url = adsurl
        link_ads.display = bool(adsurl)

        if eprint:
            link_arxiv.text = f"arXiv:{eprint}"
            link_arxiv.url = f"https://arxiv.org/abs/{eprint}"
        link_arxiv.display = bool(eprint)

        link_doi.url = f"https://doi.org/{doi}" if doi else ""
        link_doi.display = bool(doi)

        self.query_one("#detail-links").display = bool(adsurl or eprint or doi)

    def show_entry(self, entry: Entry | None) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        if entry is None:
            body.update("")
            footer.update("")
            self._set_links("", "", "")
            return
        from rich.text import Text
        content = Text()
        content.append(entry.title, style="bold")
        content.append("\n\n")
        content.append(_format_authors(entry.author), style="dim")
        content.append("   ·   ", style="dim")
        content.append(entry.year, style="green")
        abstract = entry.data.get("abstract", "")
        content.append("\n\n")
        content.append(abstract[:1000] + ("…" if len(abstract) > 1000 else ""))
        body.update(content)
        self._set_links(entry.adsurl, entry.eprint, entry.doi)
        foot = Text()
        if entry.keywords:
            foot.append(" · ".join(entry.keywords), style="dim")
            foot.append("\n\n")
        short = entry.short_key or entry.key
        foot.append(short, style="cyan")
        if short != entry.key:
            foot.append(f"  [{entry.key}]", style="dim")
        footer.update(foot)

    def show_ads_article(self, article, entry: Entry | None = None) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        if article is None:
            body.update("")
            footer.update("")
            self._set_links("", "", "")
            return
        from rich.text import Text
        title = article.title[0] if article.title else ""
        authors = article.author or []
        author_str = _format_authors(" and ".join(authors))
        abstract = article.abstract or ""
        eprint = _arxiv_id_from_identifiers(article.identifier or [])
        doi = (article.doi or [""])[0]
        content = Text()
        content.append(title, style="bold")
        content.append("\n\n")
        content.append(author_str, style="dim")
        content.append("   ·   ", style="dim")
        content.append(str(article.year or ""), style="green")
        if abstract:
            content.append("\n\n")
            content.append(abstract[:1000] + ("…" if len(abstract) > 1000 else ""))
        body.update(content)
        self._set_links(
            adsurl=f"https://ui.adsabs.harvard.edu/abs/{article.bibcode}",
            eprint=entry.eprint if entry else eprint,
            doi=entry.doi if entry else doi,
        )
        foot = Text()
        if entry:
            if entry.keywords:
                foot.append(" · ".join(entry.keywords), style="dim")
                foot.append("\n\n")
            short = entry.short_key or entry.key
            foot.append(short, style="cyan")
            if short != entry.key:
                foot.append(f"  [{entry.key}]", style="dim")
        elif article.bibcode:
            foot.append(article.bibcode, style="dim")
        footer.update(foot)

    def show_concept(self, concept: Concept | None, uat: UAT) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        footer.update("")
        self._set_links("", "", "")
        if concept is None:
            body.update("[dim]Select a concept.[/dim]")
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
        body.update("\n".join(lines))


# ── Config modal ──────────────────────────────────────────────────────────────

class ConfigModal(ModalScreen):
    DEFAULT_CSS = """
    ConfigModal { align: center middle; }
    ConfigModal Vertical {
        width: 72; height: auto;
        border: round $accent; padding: 1 2; background: $surface;
    }
    ConfigModal .cfg-row {
        height: 3;
        layout: horizontal;
        margin-bottom: 1;
    }
    ConfigModal .cfg-label {
        width: 20;
        padding: 1 2 1 0;
        color: $text-muted;
        text-align: right;
    }
    ConfigModal Input { width: 1fr; }
    ConfigModal #cfg-info { height: auto; margin-top: 1; }
    """

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Config  [dim](Enter to save · Escape to close)[/dim]")
            with Horizontal(classes="cfg-row"):
                yield Label("ADS token", classes="cfg-label")
                yield Input(placeholder="paste ADS token…", id="cfg-token")
            with Horizontal(classes="cfg-row"):
                yield Label("Unpaywall email", classes="cfg-label")
                yield Input(placeholder="your@email.com", id="cfg-email")
            yield Static("", id="cfg-info")

    def on_mount(self) -> None:
        self.query_one("#cfg-token", Input).value = get_token() or ""
        self.query_one("#cfg-email", Input).value = get_email() or ""
        self._refresh_info()
        self.query_one("#cfg-token", Input).focus()

    def _refresh_info(self) -> None:
        from rich.text import Text
        from .. import ads_client
        t = Text()
        t.append("library    ", style="dim")
        t.append(str(get_library_path()) + "\n", style="cyan")
        t.append("pdf cache  ", style="dim")
        t.append(str(PDF_CACHE_DIR) + "\n", style="cyan")
        t.append("state file ", style="dim")
        t.append(str(STATE_FILE), style="cyan")
        quota = ads_client.get_quota()
        if quota and quota.get("limit"):
            remaining = quota["remaining"]
            limit = quota["limit"]
            color = "green" if remaining > limit * 0.2 else "yellow" if remaining > 0 else "red"
            t.append("\nADS quota  ", style="dim")
            t.append(f"{remaining}/{limit}", style=color)
        self.query_one("#cfg-info", Static).update(t)

    @on(Input.Submitted, "#cfg-token")
    def _save_token(self, event: Input.Submitted) -> None:
        value = event.value.strip()
        if value:
            set_token(value)
            self.app._set_status("[green]ADS token saved.[/green]")
        self.query_one("#cfg-email", Input).focus()

    @on(Input.Submitted, "#cfg-email")
    def _save_email(self, event: Input.Submitted) -> None:
        value = event.value.strip()
        if value:
            set_email(value)
            self.app._set_status(f"[green]Email saved: {value}[/green]")
        self.dismiss()

    def on_key(self, event) -> None:
        if event.key == "escape":
            self.dismiss()


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
        t.add_column("●", key="lib", width=2)
        t.add_column("★", key="star", width=2)
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
            star = "★" if e.starred else ""
            kws = ", ".join(e.keywords[:3])
            if len(e.keywords) > 3:
                kws += "…"
            title = e.title[:52] + "…" if len(e.title) > 52 else e.title
            t.add_row(sel, cached, "●", star, e.year, e.first_author_last, title, kws, key=e.key)
        first = entries[0] if entries else None
        self._highlighted_entry = first
        self.post_message(self.EntryHighlighted(first))

    def refresh_pdf_status(self) -> None:
        from .. import pdf as _pdf
        t = self.query_one("#lib-table", DataTable)
        for e in self._all_entries:
            try:
                t.update_cell(e.key, "pdf", "↓" if _pdf.is_cached(e.key) else "")
            except Exception:
                pass

    def refresh_star_status(self) -> None:
        t = self.query_one("#lib-table", DataTable)
        for e in self._all_entries:
            try:
                t.update_cell(e.key, "star", "★" if e.starred else "")
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
        def __init__(self, article, tab_id: str) -> None:
            super().__init__()
            self.article = article
            self.tab_id = tab_id

    def __init__(self, query: str, tab_id: str) -> None:
        super().__init__(id=f"ads-view-{tab_id}")
        self.query = query
        self.tab_id = tab_id
        self._articles: list = []
        self._selected_article = None
        self._selected_bibcodes: set[str] = set()

    def compose(self) -> ComposeResult:
        yield DataTable(cursor_type="row")

    def on_mount(self) -> None:
        t = self.query_one(DataTable)
        t.add_column(" ", key="sel", width=2)
        t.add_column("↓", key="pdf", width=2)
        t.add_column("●", key="lib", width=2)
        t.add_column("★", key="star", width=2)
        t.add_column("Year", key="year", width=6)
        t.add_column("Author", key="author", width=20)
        t.add_column("Title", key="title")

    def load_articles(self, articles: list, library: Library | None) -> None:
        from .. import pdf as _pdf
        self._articles = articles
        self._selected_bibcodes.clear()
        t = self.query_one(DataTable)
        t.clear()
        for a in articles:
            cached = "↓" if _pdf.is_cached(a.bibcode) else ""
            entry = library.get_by_bibcode(a.bibcode) if library else None
            lib_icon = "●" if entry else ""
            star = "★" if entry and entry.starred else ""
            first_author = a.author[0].split(",")[0] if a.author else ""
            title = a.title[0] if a.title else ""
            short_title = title[:55] + "…" if len(title) > 55 else title
            t.add_row("", cached, lib_icon, star, str(a.year or ""), first_author, short_title, key=a.bibcode)
        first = articles[0] if articles else None
        self._selected_article = first
        self.post_message(self.ArticleHighlighted(first, self.tab_id))

    def toggle_selection(self) -> None:
        if self._selected_article is None:
            return
        bibcode = self._selected_article.bibcode
        if bibcode in self._selected_bibcodes:
            self._selected_bibcodes.discard(bibcode)
            new_val = ""
        else:
            self._selected_bibcodes.add(bibcode)
            new_val = "✓"
        try:
            self.query_one(DataTable).update_cell(bibcode, "sel", new_val)
        except Exception:
            pass

    def update_lib_status(self, library: Library | None) -> None:
        t = self.query_one(DataTable)
        for a in self._articles:
            entry = library.get_by_bibcode(a.bibcode) if library else None
            lib_icon = "●" if entry else ""
            star = "★" if entry and entry.starred else ""
            try:
                t.update_cell(a.bibcode, "lib", lib_icon)
                t.update_cell(a.bibcode, "star", star)
            except Exception:
                pass

    def refresh_pdf_status(self) -> None:
        from .. import pdf as _pdf
        t = self.query_one(DataTable)
        for a in self._articles:
            try:
                t.update_cell(a.bibcode, "pdf", "↓" if _pdf.is_cached(a.bibcode) else "")
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
        self.post_message(self.ArticleHighlighted(article, self.tab_id))


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
    #detail { width: 48; min-width: 30; }
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
        Binding("p", "download_pdf", "DL PDF"),
        Binding("o", "open_pdf", "Open PDF"),
        Binding("X", "clear_pdf", "Clear PDF"),
        Binding("/", "filter", "Filter"),
        Binding("S", "ads_search", "ADS search"),
        Binding("C", "config", "Config"),
        Binding("r", "refresh_tab", "Refresh"),
        Binding("ctrl+w", "close_tab", "Close tab", show=True),
        Binding("[", "prev_tab", "Prev tab", show=False),
        Binding("]", "next_tab", "Next tab", show=False),
        Binding("space", "toggle_select", "Select", show=False),
        Binding("s", "star", "Star", show=False),
        Binding("e", "export_selected", "Export"),
        Binding("u", "uat_browser", "UAT"),
        Binding("question_mark", "help", "Help"),
        Binding("escape", "clear_filter", "Clear", show=False),
        Binding("z", "zoom", "Zoom", show=False),
    ]

    def __init__(self) -> None:
        super().__init__()
        self._library: Library | None = None
        self._uat: UAT | None = None
        self._tab_states: list[dict] = []
        self._ads_views: dict[str, AdsView] = {}
        self._split_idx: int = 0

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
        for tab_data in self._tab_states:
            await self._create_ads_tab(tab_data, fetch=False, switch=False)
        self.call_after_refresh(self._focus_active_table)

        n = len(self._library.entries())
        uat_note = f" · UAT ({len(self._uat)})" if self._uat else ""
        token_note = "" if get_token() else "  [yellow]T: set ADS token[/yellow]"
        self._set_status(f"{n} papers{uat_note}{token_note}")

        # Restore ADS results in background after the app is displayed
        if get_token() and self._tab_states:
            self.call_after_refresh(self._restore_tabs_background)

    def _restore_tabs_background(self) -> None:
        for tab_data in self._tab_states:
            tab_id = tab_data["id"]
            ads_view = self._ads_views.get(tab_id)
            if ads_view:
                self.run_worker(self._do_refresh(ads_view, tab_data))

    # ── Tab helpers ───────────────────────────────────────────────────────────

    def _active_pane_id(self) -> str:
        return self.query_one(TabbedContent).active or ""

    def _active_ads_view(self) -> AdsView | None:
        pane_id = self._active_pane_id()
        if not pane_id or pane_id == "pane-library":
            return None
        tab_id = pane_id.removeprefix("pane-")
        return self._ads_views.get(tab_id)

    async def _create_ads_tab(self, tab_data: dict, fetch: bool = True, switch: bool = True) -> AdsView:
        tab_id = tab_data["id"]
        ads_view = AdsView(query=tab_data["query"], tab_id=tab_id)
        self._ads_views[tab_id] = ads_view
        pane = TabPane(tab_data["label"], ads_view, id=f"pane-{tab_id}")
        tc = self.query_one(TabbedContent)
        await tc.add_pane(pane)
        if switch:
            tc.active = f"pane-{tab_id}"
            self.call_after_refresh(self._focus_active_table)
        if fetch:
            await self._do_refresh(ads_view, tab_data)
        return ads_view

    async def _do_refresh(self, ads_view: AdsView, tab_data: dict) -> None:
        import asyncio
        def _is_active() -> bool:
            return self._active_ads_view() is ads_view
        if _is_active():
            self._set_status(f"Searching ADS for '{ads_view.query}'…")
        try:
            from .. import ads_client
            articles = await asyncio.to_thread(ads_client.search, ads_view.query, 20)
            ads_view.load_articles(articles, self._library)
            tab_data["bibcodes"] = [a.bibcode for a in articles]
            tab_data["refreshed"] = int(time.time())
            tabs_state.save(self._tab_states)
            ads_client.refresh_quota()
            n = len(articles)
            if _is_active():
                self._set_status(
                    f"[yellow]{n} result(s) for '{ads_view.query}'[/yellow]"
                    f"  [dim]a: add · d: remove · r: refresh · Ctrl+W: close{_quota_str()}[/dim]"
                )
        except RuntimeError as e:
            if _is_active():
                self._set_status(f"[red]{e}[/red]")

    def _reload_library(self) -> None:
        self._library = Library(root=get_library_path())
        self.query_one(LibraryView).load(self._library)
        for av in self._ads_views.values():
            av.update_lib_status(self._library)

    # ── Tab events ────────────────────────────────────────────────────────────

    @on(TabbedContent.TabActivated)
    def _on_tab_activated(self, event: TabbedContent.TabActivated) -> None:
        self.refresh_bindings()
        pane_id = self._active_pane_id()
        detail = self.query_one(DetailPanel)
        if pane_id == "pane-library":
            lib_view = self.query_one(LibraryView)
            detail.show_entry(lib_view._highlighted_entry)
            n = len(lib_view._filtered)
            total = len(lib_view._all_entries)
            shown = f" · {n} shown" if n != total else ""
            self._set_status(f"{total} papers{shown}  [dim]/ filter · Space select · e export[/dim]")
        elif pane_id:
            tab_id = pane_id.removeprefix("pane-")
            ads_view = self._ads_views.get(tab_id)
            if ads_view:
                article = ads_view._selected_article
                if article:
                    entry = self._library.get_by_bibcode(article.bibcode) if self._library else None
                    detail.show_ads_article(article, entry=entry)
                else:
                    detail.show_entry(None)
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
        if self._active_pane_id() != f"pane-{event.tab_id}":
            return
        entry = self._library.get_by_bibcode(event.article.bibcode) if self._library else None
        self.query_one(DetailPanel).show_ads_article(event.article, entry=entry)
        self.refresh_bindings()

    # ── Actions ───────────────────────────────────────────────────────────────

    def action_next_tab(self) -> None:
        self._switch_tab(+1)

    def action_prev_tab(self) -> None:
        self._switch_tab(-1)

    _PANEL_WIDTHS = [48, 64, 80, 32]

    def action_zoom(self) -> None:
        self._split_idx = (self._split_idx + 1) % len(self._PANEL_WIDTHS)
        self.query_one(DetailPanel).styles.width = self._PANEL_WIDTHS[self._split_idx]

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
        focused = self.focused
        if isinstance(focused, Input):
            return
        pane_id = self._active_pane_id()
        if pane_id == "pane-library":
            self.query_one(LibraryView).toggle_selection()
        else:
            ads_view = self._active_ads_view()
            if ads_view:
                ads_view.toggle_selection()
                self.refresh_bindings()

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

    def action_config(self) -> None:
        self.push_screen(ConfigModal())

    def action_ads_search(self) -> None:
        if not get_token():
            self.push_screen(ConfigModal())
            self._set_status("[yellow]Set your ADS token, then press S to search.[/yellow]")
            return

        async def handle(query: str) -> None:
            if not query:
                return
            tab_data = tabs_state.make_tab(query)
            self._tab_states.append(tab_data)
            await self._create_ads_tab(tab_data, fetch=True)

        self.push_screen(AdsSearchModal(), handle)

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
            if ads_view._selected_bibcodes:
                n = len(ads_view._selected_bibcodes)
                self._set_status(f"Fetching {n} paper(s)…")
                self.run_worker(self._fetch_and_add_batch(list(ads_view._selected_bibcodes), ads_view), exclusive=True)
                return
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
                    self._set_status(f"[green]Added {entry.short_key or entry.key}[/green]")
                except Exception as exc:
                    self._set_status(f"[red]{exc}[/red]")
            self.push_screen(AddModal(), handle)

    async def _fetch_and_add(self, bibcode: str, ads_view: AdsView) -> None:
        import asyncio
        try:
            from .. import ads_client
            data = await asyncio.to_thread(ads_client.fetch_bibtex, bibcode)
            if data is None:
                self._set_status(f"[red]Could not fetch {bibcode}[/red]")
                return
            entry = self._library.save_entry(data)
            self._reload_library()
            ads_view.update_lib_status(self._library)
            self.query_one(DetailPanel).show_entry(self._library.get(entry.key))
            self.refresh_bindings()
            ads_client.refresh_quota()
            self._set_status(f"[green]Added {entry.short_key or entry.key}[/green]{_quota_str()}")
        except Exception as exc:
            self._set_status(f"[red]{exc}[/red]")

    async def _fetch_and_add_batch(self, bibcodes: list[str], ads_view: AdsView) -> None:
        import asyncio
        from .. import ads_client
        added: list[str] = []
        skipped: list[str] = []
        total = len(bibcodes)
        for i, bibcode in enumerate(bibcodes):
            if self._library and self._library.has_bibcode(bibcode):
                skipped.append(bibcode)
                self._set_status(f"[dim]Skipping {bibcode} — already in library ({i+1}/{total})[/dim]")
                continue
            self._set_status(f"Fetching [{i+1}/{total}] {bibcode}…")
            try:
                data = await asyncio.to_thread(ads_client.fetch_bibtex, bibcode)
                if data is None:
                    skipped.append(bibcode)
                    self._set_status(f"[yellow]Could not fetch {bibcode} ({i+1}/{total})[/yellow]")
                    continue
                entry = self._library.save_entry(data)
                added.append(entry.key)
                self._set_status(f"[green]Added {entry.short_key or entry.key}[/green]  [{i+1}/{total}]")
            except Exception as exc:
                skipped.append(bibcode)
                self._set_status(f"[red]{bibcode}: {exc}[/red]")
        self._reload_library()
        ads_view._selected_bibcodes.clear()
        t = ads_view.query_one(DataTable)
        for a in ads_view._articles:
            try:
                t.update_cell(a.bibcode, "sel", "")
            except Exception:
                pass
        self.refresh_bindings()
        msg = f"[green]Added {len(added)} paper(s)[/green]"
        if skipped:
            msg += f"  [dim]{len(skipped)} skipped[/dim]"
        self._set_status(msg + _quota_str())

    def action_remove_paper(self) -> None:
        ads_view = self._active_ads_view()
        if ads_view is None:
            if self._active_pane_id() == "pane-library":
                entry = self.query_one(LibraryView)._highlighted_entry
                if entry and self._library:
                    self._library.remove_entry(entry.key)
                    self._reload_library()
                    self.refresh_bindings()
                    self._set_status(f"[green]Removed {entry.short_key or entry.key}[/green]")
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
        ads_view.update_lib_status(self._library)
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

    def action_download_pdf(self) -> None:
        from .. import pdf, ads_client as _ac
        ads_view = self._active_ads_view()
        if ads_view is not None:
            article = ads_view._selected_article
            if article is None:
                return
            eprint = _ac.arxiv_id_from_article(article)
            doi = (article.doi or [""])[0]
            key = article.bibcode
        else:
            entry = self.query_one(LibraryView)._highlighted_entry
            if entry is None:
                return
            eprint, doi, key = entry.eprint, entry.doi, entry.key
        if not eprint and not doi:
            self._set_status(f"[yellow]No arXiv ID or DOI for {key}[/yellow]")
            return
        self._set_status(f"Downloading {key}…")
        self.run_worker(self._do_download_pdf(key, eprint, doi, ads_view), exclusive=True)

    async def _do_download_pdf(self, key: str, eprint: str, doi: str,
                                ads_view: "AdsView | None") -> None:
        import asyncio
        from .. import pdf
        path = await asyncio.to_thread(pdf.fetch, key, eprint=eprint, doi=doi, force=True)
        if path is None:
            self._set_status(f"[red]No PDF found for {key}[/red]")
        else:
            sz = path.stat().st_size // 1024
            self._set_status(f"[green]Downloaded {key}  ({sz} KB)[/green]")
            if ads_view is not None:
                ads_view.refresh_pdf_status()
            else:
                self.query_one(LibraryView).refresh_pdf_status()
            self.refresh_bindings()

    def action_open_pdf(self) -> None:
        from .. import pdf, ads_client as _ac
        ads_view = self._active_ads_view()
        if ads_view is not None:
            article = ads_view._selected_article
            if article is None:
                self._set_status("[yellow]Select an article first.[/yellow]")
                return
            eprint = _ac.arxiv_id_from_article(article)
            doi = (article.doi or [""])[0]
            key = article.bibcode
        else:
            lib_view = self.query_one(LibraryView)
            entry = lib_view._highlighted_entry
            if entry is None:
                self._set_status("[yellow]Select a paper first.[/yellow]")
                return
            eprint = entry.eprint
            doi = entry.doi
            key = entry.key
        if not eprint and not doi:
            self._set_status(f"[yellow]No arXiv ID or DOI for {key}[/yellow]")
            return
        if pdf.is_cached(key):
            self._set_status(f"Opening {key}…")
        elif doi:
            self._set_status(f"Fetching {key} via Unpaywall…")
        else:
            self._set_status(f"Fetching {key} from arXiv:{eprint}…")
        self.run_worker(self._do_open_pdf(key, eprint, doi, ads_view), exclusive=True)

    async def _do_open_pdf(self, key: str, eprint: str, doi: str, ads_view: "AdsView | None") -> None:
        import asyncio
        from .. import pdf
        opened = await asyncio.to_thread(pdf.open_pdf, key, eprint=eprint, doi=doi)
        if not opened:
            self._set_status(f"[red]No open-access PDF found for {key}[/red]")
        else:
            self._set_status(f"[green]Opened {key}[/green]")
            if ads_view is not None:
                ads_view.refresh_pdf_status()
            else:
                self.query_one(LibraryView).refresh_pdf_status()
            self.refresh_bindings()

    def action_star(self) -> None:
        if self._library is None:
            return
        pane_id = self._active_pane_id()
        if pane_id == "pane-library":
            lib_view = self.query_one(LibraryView)
            entry = lib_view._highlighted_entry
            if entry is None:
                return
            new_starred = not entry.starred
            self._library.set_starred(entry.key, new_starred)
            lib_view.refresh_star_status()
            self._set_status(f"{'★ Starred' if new_starred else 'Unstarred'} {entry.short_key or entry.key}")
        else:
            ads_view = self._active_ads_view()
            if ads_view is None:
                return
            article = ads_view._selected_article
            if article is None:
                return
            entry = self._library.get_by_bibcode(article.bibcode)
            if entry is None:
                self._set_status("[yellow]Not in library — add it first to star it.[/yellow]")
                return
            new_starred = not entry.starred
            self._library.set_starred(entry.key, new_starred)
            ads_view.update_lib_status(self._library)
            self._set_status(f"{'★ Starred' if new_starred else 'Unstarred'} {entry.short_key or entry.key}")

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
        on_library = pane_id == "pane-library"
        ads_view = self._active_ads_view()

        # Actions only valid on ADS tabs
        if action in ("refresh_tab", "close_tab"):
            return not on_library

        # Actions only valid on Library tab
        if action in ("filter", "export_selected"):
            return on_library

        if action == "toggle_select":
            if on_library:
                return True
            if ads_view:
                return ads_view._selected_article is not None
            return False

        if action == "star":
            if on_library:
                lib_view = self.query_one(LibraryView)
                return lib_view._highlighted_entry is not None
            if ads_view:
                a = ads_view._selected_article
                return a is not None and self._library is not None and self._library.has_bibcode(a.bibcode)
            return False

        if action == "clear_pdf":
            if on_library:
                entry = self.query_one(LibraryView)._highlighted_entry
                return entry is not None and _pdf.is_cached(entry.key)
            if ads_view and ads_view._selected_article:
                return _pdf.is_cached(ads_view._selected_article.bibcode)
            return False

        if action in ("open_pdf", "download_pdf"):
            if on_library:
                entry = self.query_one(LibraryView)._highlighted_entry
                return entry is not None and bool(entry.eprint or entry.doi)
            if ads_view:
                a = ads_view._selected_article
                return a is not None and bool(
                    a.identifier or (a.doi and a.doi[0])
                )
            return False

        if action == "add_paper":
            if on_library:
                return False
            if ads_view is not None:
                if ads_view._selected_bibcodes:
                    return any(
                        not (self._library and self._library.has_bibcode(bc))
                        for bc in ads_view._selected_bibcodes
                    )
                article = ads_view._selected_article
                if article and self._library:
                    return self._library.get_by_bibcode(article.bibcode) is None
            return True

        if action == "remove_paper":
            if on_library:
                lib_view = self.query_one(LibraryView)
                return lib_view._highlighted_entry is not None
            if ads_view is not None:
                article = ads_view._selected_article
                if article and self._library:
                    return self._library.get_by_bibcode(article.bibcode) is not None
            return False

        return True

    def _set_status(self, msg: str) -> None:
        self.query_one("#status-bar", Static).update(msg)


# ── Helpers ───────────────────────────────────────────────────────────────────


def _format_authors(raw: str, max_count: int = 5) -> str:
    if not raw:
        return ""
    authors = [a.strip() for a in raw.split(" and ")]
    parts = []
    for a in authors[:max_count]:
        if "," in a:
            last, rest = a.split(",", 1)
            initial = rest.strip()[:1]
            parts.append(f"{last.strip()}, {initial}." if initial else last.strip())
        else:
            parts.append(a)
    result = "; ".join(parts)
    if len(authors) > max_count:
        result += " et al."
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
