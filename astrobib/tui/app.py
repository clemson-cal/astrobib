from __future__ import annotations

import threading
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

from ..library import Entry, Library, MergedLibrary
from ..state import (
    UAT_CACHE, PDF_CACHE_DIR, STATE_FILE,
    find_manuscript_db, get_library_path, get_token, set_token,
)
from ..uat import UAT, Concept, get_uat
from . import tabs_state


# ── PDF action button ─────────────────────────────────────────────────────────

class PdfButton(Static, can_focus=True):
    """Clickable download trigger in the detail panel."""
    DEFAULT_CSS = """
    PdfButton {
        width: auto;
        padding: 0 1;
        margin-right: 1;
        background: $panel-lighten-1;
        color: $text-muted;
        &:hover { background: $panel-lighten-2; color: $text; }
        &:focus { background: $panel-lighten-2; }
    }
    PdfButton.arxiv  { color: cyan; }
    PdfButton.oa     { color: cyan; }
    PdfButton.browser { color: yellow; }
    PdfButton.open   { color: green; }
    PdfButton.clear  { color: $text-muted; }
    """

    class Clicked(Message):
        def __init__(self, source: str) -> None:
            super().__init__()
            self.source = source

    def __init__(self, label: str, source: str, **kwargs) -> None:
        super().__init__(label, **kwargs)
        self._source = source

    def on_click(self) -> None:
        self.post_message(self.Clicked(self._source))


# ── Detail panel ──────────────────────────────────────────────────────────────

# Style for the trailing hash portion of a cite key that citations don't
# need. The dim attribute survives the DataTable cursor row's color
# override, so the suffix stays faded even on the highlighted row.
_HASH_STYLE = "dim grey35"


def _append_key(text, entry: Entry) -> None:
    """Append a cite key: short prefix in cyan, unused hash suffix very dim."""
    short = entry.short_key or entry.key
    text.append(short, style="cyan")
    if short != entry.key:
        text.append(entry.key[len(short):], style=_HASH_STYLE)


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
        padding: 1 2;
        layout: horizontal;
        margin-top: 1;
        border-top: solid $panel-lighten-1;
        border-bottom: solid $panel-lighten-1;
    }
    DetailPanel Link {
        width: auto;
        padding-right: 2;
        color: cyan;
        text-style: none;
        &:hover { text-style: underline; }
    }
    DetailPanel #pdf-sources {
        height: auto;
        padding: 1 2 0 2;
        layout: horizontal;
    }
    DetailPanel #pdf-status {
        height: 1;
        padding: 0 2;
    }
    DetailPanel #detail-footer {
        height: auto;
        padding: 1 2 1 2;
        margin-top: 1;
        border-top: solid $panel-lighten-1;
    }
    """

    def compose(self) -> ComposeResult:
        yield Static("", id="detail-body")
        with Horizontal(id="detail-links"):
            yield Link("ADS", url="", id="link-ads")
            yield Link("", url="", id="link-arxiv")
            yield Link("DOI", url="", id="link-doi")
        with Horizontal(id="pdf-sources"):
            yield PdfButton("arXiv ↓", source="arxiv", id="pdf-btn-arxiv", classes="arxiv")
            yield PdfButton("ADS OA ↓", source="oa", id="pdf-btn-oa", classes="oa")
            yield PdfButton("browser ↓", source="browser", id="pdf-btn-browser", classes="browser")
            yield PdfButton("Open ↗", source="open", id="pdf-btn-open", classes="open")
            yield PdfButton("Clear ✕", source="clear", id="pdf-btn-clear", classes="clear")
        yield Static("", id="pdf-status")
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

    def _update_pdf_buttons(self, *, has_eprint: bool, has_adsurl: bool, has_doi: bool,
                             cached: bool = False) -> None:
        self.query_one("#pdf-btn-arxiv").display = has_eprint and not cached
        self.query_one("#pdf-btn-oa").display = has_adsurl and not cached
        self.query_one("#pdf-btn-browser").display = (has_doi or has_adsurl) and not cached
        self.query_one("#pdf-btn-open").display = cached
        self.query_one("#pdf-btn-clear").display = cached
        has_any = has_eprint or has_adsurl or has_doi or cached
        self.query_one("#pdf-sources").display = has_any
        self.query_one("#pdf-status", Static).update("")

    def set_pdf_status(self, text: str) -> None:
        self.query_one("#pdf-status", Static).update(text)

    def show_entry(self, entry: Entry | None) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        if entry is None:
            body.update("")
            footer.update("")
            self._set_links("", "", "")
            self._update_pdf_buttons(has_eprint=False, has_adsurl=False, has_doi=False)
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
        from .. import pdf as _pdf
        self._set_links(entry.adsurl, entry.eprint, entry.doi)
        self._update_pdf_buttons(
            has_eprint=bool(entry.eprint),
            has_adsurl=bool(entry.adsurl),
            has_doi=bool(entry.doi),
            cached=_pdf.is_cached(entry.key),
        )
        foot = Text()
        if entry.keywords:
            foot.append(" · ".join(entry.keywords), style="dim")
            foot.append("\n\n")
        _append_key(foot, entry)
        footer.update(foot)

    def show_ads_article(self, article, entry: Entry | None = None) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        if article is None:
            body.update("")
            footer.update("")
            self._set_links("", "", "")
            self._update_pdf_buttons(has_eprint=False, has_adsurl=False, has_doi=False)
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
        cites = getattr(article, "_raw", {}).get("citation_count")
        if cites:
            content.append("   ·   ", style="dim")
            content.append(f"cited by {cites}", style="dim")
        if abstract:
            content.append("\n\n")
            content.append(abstract[:1000] + ("…" if len(abstract) > 1000 else ""))
        body.update(content)
        self._set_links(
            adsurl=f"https://ui.adsabs.harvard.edu/abs/{article.bibcode}",
            eprint=entry.eprint if entry else eprint,
            doi=entry.doi if entry else doi,
        )
        eff_eprint = entry.eprint if entry else eprint
        eff_doi = entry.doi if entry else doi
        cache_key = entry.key if entry else article.bibcode
        from .. import pdf as _pdf
        self._update_pdf_buttons(
            has_eprint=bool(eff_eprint),
            has_adsurl=bool(article.bibcode),
            has_doi=bool(eff_doi),
            cached=_pdf.is_cached(cache_key),
        )
        foot = Text()
        if entry:
            if entry.keywords:
                foot.append(" · ".join(entry.keywords), style="dim")
                foot.append("\n\n")
            _append_key(foot, entry)
        elif article.bibcode:
            foot.append(article.bibcode, style="dim")
        footer.update(foot)

    def show_ambiguous_key(self, key: str, matches: list[Entry]) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        lines = [f"[bold magenta]{key}[/bold magenta]\n",
                 "[dim]Ambiguous cite key — matches several entries:[/dim]\n"]
        for m in matches[:8]:
            lines.append(f"  [cyan]{m.key}[/cyan]  [dim]{m.title[:48]}[/dim]")
        lines.append("\n[dim]Lengthen the key in the .tex source until it is unique.[/dim]")
        body.update("\n".join(lines))
        footer.update("")
        self._set_links("", "", "")
        self._update_pdf_buttons(has_eprint=False, has_adsurl=False, has_doi=False)

    def show_missing_key(self, key: str) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        body.update(
            f"[bold red]{key}[/bold red]\n\n"
            "[dim]Unknown cite key — not in the manuscript database or your "
            "personal library.\n\nFix the key in the .tex source, or press "
            "[bold]S[/bold] to search ADS and import the paper.[/dim]"
        )
        footer.update("")
        self._set_links("", "", "")
        self._update_pdf_buttons(has_eprint=False, has_adsurl=False, has_doi=False)

    def show_concept(self, concept: Concept | None, uat: UAT) -> None:
        body = self.query_one("#detail-body", Static)
        footer = self.query_one("#detail-footer", Static)
        footer.update("")
        self._set_links("", "", "")
        self._update_pdf_buttons(has_eprint=False, has_adsurl=False, has_doi=False)
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
            yield Static("", id="cfg-info")

    def on_mount(self) -> None:
        self.query_one("#cfg-token", Input).value = get_token() or ""
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
        self._library: MergedLibrary | None = None
        self._ms_only: bool = False

    def compose(self) -> ComposeResult:
        yield Input(placeholder="Filter by author, title, key, or keyword…", id="lib-filter")
        yield DataTable(id="lib-table", cursor_type="row")

    def on_mount(self) -> None:
        t = self.query_one("#lib-table", DataTable)
        t.add_column(" ", key="sel", width=2)
        t.add_column("↓", key="pdf", width=2)
        t.add_column("◆", key="lib", width=2)
        t.add_column("★", key="star", width=2)
        t.add_column("Year", key="year", width=6)
        t.add_column("Author", key="author", width=20)
        t.add_column("Title", key="title")
        t.add_column("Keywords", key="keywords", width=28)

    def load(self, library: MergedLibrary | None) -> None:
        self._library = library
        self._all_entries = library.entries() if library else []
        self._selected_keys.clear()
        filter_val = self.query_one("#lib-filter", Input).value
        self._apply_filter(filter_val)

    def toggle_ms_only(self) -> bool:
        """Toggle hiding entries that are not in the manuscript db."""
        self._ms_only = not self._ms_only
        self._apply_filter(self.query_one("#lib-filter", Input).value)
        return self._ms_only

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
        pool = self._all_entries
        if self._ms_only and self._library is not None:
            pool = [e for e in pool if self._library.in_manuscript(e.key)]
        if q:
            self._filtered = [
                e for e in pool
                if q in e.title.lower()
                or q in e.author.lower()
                or q in e.key.lower()
                or any(q in kw.lower() for kw in e.keywords)
            ]
        else:
            self._filtered = list(pool)
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
            in_ms = "◆" if self._library and self._library.in_manuscript(e.key) else ""
            star = "★" if e.starred else ""
            kws = ", ".join(e.keywords[:3])
            if len(e.keywords) > 3:
                kws += "…"
            title = e.title[:52] + "…" if len(e.title) > 52 else e.title
            t.add_row(sel, cached, in_ms, star, e.year, e.first_author_last, title, kws, key=e.key)
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


# ── Manuscript view ───────────────────────────────────────────────────────────

class ManuscriptView(Vertical):
    """Health dashboard for the active manuscript database.

    Rows are the union of cite keys found in the manuscript's .tex files
    and entries in its bib/ directory:
      ok       — cited and in bib/
      library  — cited, not in bib/, but in the personal library (m to add)
      missing  — cited, found nowhere
      uncited  — in bib/ but cited by nothing
    """

    DEFAULT_CSS = """
    ManuscriptView { height: 100%; }
    ManuscriptView DataTable { height: 100%; }
    """

    class KeyHighlighted(Message):
        def __init__(self, key: str | None, entry: Entry | None, state: str) -> None:
            super().__init__()
            self.key = key
            self.entry = entry
            self.state = state

    _STATE_ORDER = {"missing": 0, "ambiguous": 1, "library": 2, "uncited": 3, "ok": 4}
    _STATE_STYLE = {"missing": "red", "ambiguous": "magenta", "library": "yellow",
                    "uncited": "cyan", "ok": ""}
    _STATE_LABEL = {"missing": "missing", "ambiguous": "ambiguous",
                    "library": "in library", "uncited": "uncited", "ok": ""}
    _STATE_ICON = {"missing": "✗", "ambiguous": "≈", "library": "○", "uncited": "·", "ok": "◆"}

    def __init__(self) -> None:
        super().__init__(id="manuscript-view")
        self._rows: list[tuple[str, str, Entry | None]] = []
        self._highlighted_key: str | None = None
        self._highlighted_state: str = "ok"
        self._highlighted_entry: Entry | None = None

    def compose(self) -> ComposeResult:
        yield DataTable(id="ms-table", cursor_type="row")

    def on_mount(self) -> None:
        t = self.query_one("#ms-table", DataTable)
        t.add_column(" ", key="state", width=2)
        t.add_column("Status", key="status", width=10)
        t.add_column("Key", key="key", width=24)
        t.add_column("Year", key="year", width=6)
        t.add_column("Author", key="author", width=20)
        t.add_column("Title", key="title")

    def load(self, library: MergedLibrary | None, cited_keys: set[str]) -> None:
        ms = library.manuscript if library else None
        rows: list[tuple[str, str, Entry | None]] = []
        targets: set[str] = set()
        for cited in cited_keys:
            state, entry = library.resolve_citation(cited) if library else ("missing", None)
            if entry is not None:
                targets.add(entry.key)
            rows.append((cited, state, entry))
        if ms is not None:
            for e in ms.entries():
                if e.key not in targets:
                    rows.append((e.key, "uncited", (library.get(e.key) if library else e)))
        rows.sort(key=lambda r: (self._STATE_ORDER[r[1]], r[0].lower()))
        self._rows = rows
        self._refresh_table()

    def counts(self) -> dict[str, int]:
        c = {"ok": 0, "library": 0, "missing": 0, "ambiguous": 0, "uncited": 0}
        for _, state, _ in self._rows:
            c[state] += 1
        return c

    def _key_cell(self, key_str: str, entry: Entry | None, style: str):
        """Render a key with the portion beyond the shortest unambiguous
        prefix very dim — the bright part is all a citation needs."""
        from rich.text import Text
        if entry is not None:
            short = entry.short_key or entry.key
            if key_str.startswith(short) and len(key_str) > len(short):
                t = Text(key_str[:len(short)], style=style or "")
                t.append(key_str[len(short):], style=_HASH_STYLE)
                return t
        return Text(key_str, style=style) if style else Text(key_str)

    def _refresh_table(self) -> None:
        from rich.text import Text
        t = self.query_one("#ms-table", DataTable)
        prev_key = self._highlighted_key
        t.clear()
        for key, state, entry in self._rows:
            style = self._STATE_STYLE[state]
            cell = (lambda s, _st=style: Text(s, style=_st) if _st else Text(s))
            title = entry.title if entry else ""
            title = title[:52] + "…" if len(title) > 52 else title
            t.add_row(
                cell(self._STATE_ICON[state]), cell(self._STATE_LABEL[state]),
                self._key_cell(key, entry, style), cell(entry.year if entry else ""),
                cell(entry.first_author_last if entry else ""), cell(title),
                key=key,
            )
        row = None
        if prev_key is not None:
            row = next((r for r in self._rows if r[0] == prev_key), None)
            if row is not None:
                try:
                    t.move_cursor(row=t.get_row_index(prev_key))
                except Exception:
                    row = None
        if row is None:
            row = self._rows[0] if self._rows else (None, "ok", None)
        self._highlighted_key, self._highlighted_state = row[0], row[1]
        self._highlighted_entry = row[2]
        self.post_message(self.KeyHighlighted(row[0], row[2], row[1]))

    @on(DataTable.RowHighlighted, "#ms-table")
    @on(DataTable.RowSelected, "#ms-table")
    def _on_row_event(self, event) -> None:
        key = event.row_key.value if event.row_key else None
        if not key:
            return
        row = next((r for r in self._rows if r[0] == key), None)
        if row is None:
            return
        self._highlighted_key, self._highlighted_state = row[0], row[1]
        self._highlighted_entry = row[2]
        self.post_message(self.KeyHighlighted(row[0], row[2], row[1]))


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

    class ImportRequested(Message):
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
    def _on_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        key = event.row_key.value if event.row_key else None
        if not key:
            return
        article = next((a for a in self._articles if a.bibcode == key), None)
        self._selected_article = article
        self.post_message(self.ArticleHighlighted(article, self.tab_id))

    @on(DataTable.RowSelected)
    def _on_row_selected(self, event: DataTable.RowSelected) -> None:
        key = event.row_key.value if event.row_key else None
        if not key:
            return
        article = next((a for a in self._articles if a.bibcode == key), None)
        self._selected_article = article
        self.post_message(self.ArticleHighlighted(article, self.tab_id))
        self.post_message(self.ImportRequested(article, self.tab_id))


# ── Modals ────────────────────────────────────────────────────────────────────

class AdsSearchModal(ModalScreen["tuple[str, int] | None"]):
    DEFAULT_CSS = """
    AdsSearchModal { align: center middle; }
    AdsSearchModal Vertical {
        width: 64; height: auto;
        border: round $accent; padding: 1 2; background: $surface;
    }
    AdsSearchModal Horizontal { height: auto; margin-top: 1; }
    AdsSearchModal .count-label { padding: 1 1 0 0; color: $text-muted; }
    AdsSearchModal #ads-count { width: 10; }
    """

    def __init__(self, initial: str = "", title: str = "New ADS search",
                 limit: int = tabs_state.DEFAULT_LIMIT) -> None:
        super().__init__()
        self._initial = initial
        self._title = title
        self._limit = limit

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(f"{self._title}  [dim](Enter / Escape)[/dim]")
            yield Input(value=self._initial,
                        placeholder="author, title, or topic…", id="ads-input")
            with Horizontal():
                yield Label("Results:", classes="count-label")
                yield Input(value=str(self._limit), type="integer", id="ads-count")

    def on_mount(self) -> None:
        self.query_one("#ads-input", Input).focus()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        query = self.query_one("#ads-input", Input).value.strip()
        try:
            limit = int(self.query_one("#ads-count", Input).value)
        except ValueError:
            limit = tabs_state.DEFAULT_LIMIT
        limit = max(1, min(2000, limit))
        self.dismiss((query, limit))

    def on_key(self, event) -> None:
        if event.key == "escape":
            self.dismiss(None)


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

class AstrobibApp(App):
    TITLE = "astrobib"
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
        # Footer-visible core set: fixed positions, greyed out when unavailable
        Binding("i", "add_paper", "Import"),
        Binding("d", "remove_paper", "Remove"),
        Binding("p", "download_pdf", "DL PDF"),
        Binding("o", "open_pdf", "Open PDF"),
        Binding("/", "filter", "Filter/Query"),
        Binding("S", "ads_search", "ADS search"),
        Binding("r", "refresh_tab", "Refresh"),
        Binding("question_mark", "help", "Help"),
        Binding("q", "quit", "Quit"),
        # Hidden bindings: fully functional, documented in help (?)
        Binding("B", "browser_pdf", "Browser DL", show=False),
        Binding("X", "clear_pdf", "Clear PDF", show=False),
        Binding("C", "config", "Config", show=False),
        Binding("e", "export_selected", "Export", show=False),
        Binding("u", "uat_browser", "UAT", show=False),
        Binding("ctrl+w", "close_tab", "Close tab", show=False),
        Binding("plus,equals_sign", "more_results", show=False),
        Binding("minus", "fewer_results", show=False),
        Binding("[", "prev_tab", "Prev tab", show=False),
        Binding("]", "next_tab", "Next tab", show=False),
        Binding("space", "toggle_select", "Select", show=False),
        Binding("s", "star", "Star", show=False),
        Binding("R", "references", "References", show=False),
        Binding("c", "citations", "Citations", show=False),
        Binding("m", "toggle_manuscript", "Manuscript ±", show=False),
        Binding("M", "toggle_ms_only", "Ms-only view", show=False),
        Binding("y", "copy_key(False)", "Copy key", show=False),
        Binding("Y", "copy_key(True)", "Copy full key", show=False),
        Binding("escape", "clear_filter", "Clear", show=False),
        Binding("z", "zoom", "Zoom", show=False),
    ]

    def __init__(self) -> None:
        super().__init__()
        self._library: MergedLibrary | None = None
        self._ms_root: "Path | None" = find_manuscript_db()
        self._cited_keys: set[str] = set()
        self._ms_tex_mtimes: dict[str, float] = {}
        self._ms_bib_mtime: float = 0.0
        self._uat: UAT | None = None
        self._tab_states: list[dict] = []
        self._ads_views: dict[str, AdsView] = {}
        self._split_idx: int = 0
        self._poll_cancel: threading.Event | None = None

    def compose(self) -> ComposeResult:
        yield Header()
        with Horizontal(id="body"):
            with TabbedContent(id="tabs"):
                with TabPane("Library", id="pane-library"):
                    yield LibraryView()
                if self._ms_root:
                    with TabPane("Manuscript", id="pane-manuscript"):
                        yield ManuscriptView()
            yield DetailPanel(id="detail")
        yield Static("", id="status-bar")
        yield Footer()

    def _make_library(self) -> MergedLibrary:
        personal = Library(root=get_library_path())
        ms = Library(root=self._ms_root) if self._ms_root else None
        return MergedLibrary(personal=personal, manuscript=ms)

    async def on_mount(self) -> None:
        self._uat = get_uat(UAT_CACHE, auto_fetch=False)

        if self._ms_root:
            self.sub_title = f"ms: {self._ms_root.name}"
        self._library = self._make_library()
        self.query_one(LibraryView).load(self._library)

        if self._ms_root:
            bib_dir = self._ms_root / "bib"
            self._ms_bib_mtime = bib_dir.stat().st_mtime if bib_dir.exists() else 0.0
            self._poll_manuscript()
            self.set_interval(2.0, self._poll_manuscript)

        self._tab_states = tabs_state.load(self._ms_root)
        for tab_data in self._tab_states:
            await self._create_ads_tab(tab_data, fetch=False, switch=False)
        self.call_after_refresh(self._focus_active_table)

        n = len(self._library.entries())
        uat_note = f" · UAT ({len(self._uat)})" if self._uat else ""
        token_note = "" if get_token() else "  [yellow]T: set ADS token[/yellow]"
        ms_note = f" · [cyan]ms: {self._ms_root.name}[/cyan]" if self._ms_root else ""
        self._set_status(f"{n} papers{ms_note}{uat_note}{token_note}")

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
        limit = tab_data.get("limit", tabs_state.DEFAULT_LIMIT)
        if _is_active():
            self._set_status(f"Searching ADS for '{ads_view.query}' (n={limit})…")
        try:
            from .. import ads_client
            articles = await asyncio.to_thread(ads_client.search, ads_view.query, limit)
            ads_view.load_articles(articles, self._library)
            tab_data["bibcodes"] = [a.bibcode for a in articles]
            tab_data["refreshed"] = int(time.time())
            tabs_state.save(self._tab_states, self._ms_root)
            ads_client.refresh_quota()
            n = len(articles)
            if _is_active():
                self._set_status(_ads_tab_status(ads_view.query, n, limit))
        except Exception as e:
            if _is_active():
                self._set_status(f"[red]ADS search failed: {e}[/red]")

    def _reload_library(self) -> None:
        self._library = self._make_library()
        self.query_one(LibraryView).load(self._library)
        for av in self._ads_views.values():
            av.update_lib_status(self._library)
        self._refresh_manuscript_view()

    # ── Manuscript watching ───────────────────────────────────────────────────

    def _poll_manuscript(self) -> None:
        """Watch the manuscript's .tex files and bib/ for changes (2 s interval)."""
        if self._ms_root is None or self._library is None:
            return
        from ..export import manuscript_tex_files, scan_tex_files
        files = manuscript_tex_files(self._ms_root)
        tex_mtimes: dict[str, float] = {}
        for f in files:
            try:
                tex_mtimes[str(f)] = f.stat().st_mtime
            except OSError:
                pass
        bib_dir = self._ms_root / "bib"
        bib_mtime = bib_dir.stat().st_mtime if bib_dir.exists() else 0.0
        tex_changed = tex_mtimes != self._ms_tex_mtimes
        bib_changed = bib_mtime != self._ms_bib_mtime
        if not tex_changed and not bib_changed:
            return
        self._ms_tex_mtimes = tex_mtimes
        self._ms_bib_mtime = bib_mtime
        if tex_changed:
            self._cited_keys = scan_tex_files(files)
        if bib_changed:
            self._reload_library()  # refreshes the manuscript view too
        else:
            self._refresh_manuscript_view()

    def _refresh_manuscript_view(self) -> None:
        if self._ms_root is None:
            return
        try:
            ms_view = self.query_one(ManuscriptView)
        except Exception:
            return
        ms_view.load(self._library, self._cited_keys)
        self._write_refs_bib()
        if self._active_pane_id() == "pane-manuscript":
            self._set_status(self._manuscript_status(ms_view))
        self.refresh_bindings()

    def _manuscript_status(self, ms_view: ManuscriptView) -> str:
        c = ms_view.counts()
        cited = c["ok"] + c["library"] + c["missing"] + c["ambiguous"]
        parts = [f"{cited} cited"]
        if c["missing"]:
            parts.append(f"[red]{c['missing']} missing[/red]")
        if c["ambiguous"]:
            parts.append(f"[magenta]{c['ambiguous']} ambiguous[/magenta]")
        if c["library"]:
            parts.append(f"[yellow]{c['library']} in library — m to add[/yellow]")
        if c["uncited"]:
            parts.append(f"[cyan]{c['uncited']} uncited[/cyan]")
        return " · ".join(parts) + f"  [dim]· ms: {self._ms_root.name}[/dim]"

    def _write_refs_bib(self) -> None:
        """Regenerate refs.bib from cited manuscript entries, only on change.

        Each entry is emitted under the string actually cited in the .tex
        (full key or unambiguous prefix), so hash suffixes stay out of the
        manuscript.
        """
        from ..library import format_bib_entry
        ms = self._library.manuscript if self._library else None
        if ms is None or self._ms_root is None:
            return
        blocks: list[str] = []
        for cited in sorted(self._cited_keys):
            state, entry = self._library.resolve_citation(cited)
            if state != "ok" or entry is None:
                continue
            data = dict((ms.get(entry.key) or entry).data)
            data["ID"] = cited
            blocks.append(format_bib_entry(data))
        content = "\n".join(blocks)
        out = self._ms_root / "refs.bib"
        try:
            if not out.exists() or out.read_text() != content:
                out.write_text(content)
        except OSError:
            pass

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
        elif pane_id == "pane-manuscript":
            ms_view = self.query_one(ManuscriptView)
            key = ms_view._highlighted_key
            entry = ms_view._highlighted_entry
            if entry is not None:
                detail.show_entry(entry)
            elif key and ms_view._highlighted_state == "ambiguous" and self._library:
                detail.show_ambiguous_key(key, self._library.possible_matches(key))
            elif key:
                detail.show_missing_key(key)
            else:
                detail.show_entry(None)
            self._set_status(self._manuscript_status(ms_view))
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
                tab_data = next((t for t in self._tab_states if t["id"] == ads_view.tab_id), {})
                limit = tab_data.get("limit", tabs_state.DEFAULT_LIMIT)
                if not ads_view._articles:
                    self._set_status(
                        f"[dim]'{ads_view.query}' — press [bold]r[/bold] to load results[/dim]"
                    )
                else:
                    n = len(ads_view._articles)
                    self._set_status(_ads_tab_status(ads_view.query, n, limit))

    # ── Message handlers ──────────────────────────────────────────────────────

    @on(LibraryView.EntryHighlighted)
    def _on_entry_highlighted(self, event: LibraryView.EntryHighlighted) -> None:
        if self._active_pane_id() == "pane-library":
            self.query_one(DetailPanel).show_entry(event.entry)
            self.refresh_bindings()

    @on(ManuscriptView.KeyHighlighted)
    def _on_ms_key_highlighted(self, event: ManuscriptView.KeyHighlighted) -> None:
        if self._active_pane_id() != "pane-manuscript":
            return
        detail = self.query_one(DetailPanel)
        if event.entry is not None:
            detail.show_entry(event.entry)
        elif event.key and event.state == "ambiguous" and self._library:
            detail.show_ambiguous_key(event.key, self._library.possible_matches(event.key))
        elif event.key:
            detail.show_missing_key(event.key)
        else:
            detail.show_entry(None)
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
        ads_view = self._ads_views.get(event.tab_id)
        if ads_view:
            tab_data = next((t for t in self._tab_states if t["id"] == event.tab_id), {})
            limit = tab_data.get("limit", tabs_state.DEFAULT_LIMIT)
            status = _ads_tab_status(ads_view.query, len(ads_view._articles), limit)
            if entry is None:
                status += "  [dim]⏎ Import[/dim]"
            self._set_status(status)

    @on(AdsView.ImportRequested)
    def _on_import_requested(self, event: AdsView.ImportRequested) -> None:
        if self._active_pane_id() != f"pane-{event.tab_id}":
            return
        if event.article and self._library and not self._library.has_bibcode(event.article.bibcode):
            self.action_add_paper()

    @on(PdfButton.Clicked)
    def _on_pdf_button_clicked(self, event: PdfButton.Clicked) -> None:
        from .. import pdf, ads_client as _ac
        ads_view = self._active_ads_view()
        if ads_view is not None:
            article = ads_view._selected_article
            if article is None:
                return
            eprint = _ac.arxiv_id_from_article(article)
            doi = (article.doi or [""])[0]
            key = article.bibcode
            adsurl = f"https://ui.adsabs.harvard.edu/abs/{article.bibcode}"
        else:
            entry = self.query_one(LibraryView)._highlighted_entry
            if entry is None:
                return
            eprint, doi, key, adsurl = entry.eprint, entry.doi, entry.key, entry.adsurl

        if event.source == "open":
            self.action_open_pdf()
        elif event.source == "clear":
            self.action_clear_pdf()
        elif event.source == "browser":
            self._cancel_poll()
            self.run_worker(self._do_browser_pdf(key, doi, adsurl, ads_view), exclusive=True)
        else:
            self._cancel_poll()
            source = event.source if event.source in ("arxiv", "oa") else "auto"
            self._set_status(f"Downloading {key} ({event.source})…")
            self.run_worker(
                self._do_download_pdf(key, eprint, doi, adsurl, ads_view, source=source),
                exclusive=True,
            )

    def _cancel_poll(self) -> None:
        if self._poll_cancel is not None:
            self._poll_cancel.set()
            self._poll_cancel = None

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
        elif pane_id == "pane-manuscript":
            try:
                self.query_one(ManuscriptView).query_one(DataTable).focus()
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
        ads_view = self._active_ads_view()
        if ads_view is not None:
            self._edit_tab_query(ads_view)
            return
        if self._active_pane_id() == "pane-library":
            self.query_one(LibraryView).focus_filter()

    def _edit_tab_query(self, ads_view: AdsView) -> None:
        tab_data = next((t for t in self._tab_states if t["id"] == ads_view.tab_id), None)
        if tab_data is None:
            return

        async def handle(result: "tuple[str, int] | None") -> None:
            if not result or not result[0]:
                return
            query, limit = result
            if query == tab_data["query"] and limit == tab_data.get("limit"):
                return
            if query != tab_data["query"]:
                tab_data["label"] = tabs_state.short_label(query)
                self.query_one(TabbedContent).get_tab(f"pane-{ads_view.tab_id}").label = tab_data["label"]
            tab_data["query"] = query
            tab_data["limit"] = limit
            ads_view.query = query
            tabs_state.save(self._tab_states, self._ms_root)
            await self._do_refresh(ads_view, tab_data)

        self.push_screen(
            AdsSearchModal(initial=tab_data["query"], title="Edit ADS query",
                           limit=tab_data.get("limit", tabs_state.DEFAULT_LIMIT)),
            handle,
        )

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
        output = Path.cwd() / "astrobib-export.bib"
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

        async def handle(result: "tuple[str, int] | None") -> None:
            if not result or not result[0]:
                return
            query, limit = result
            from .. import ads_client
            bibcode = ads_client.bibcode_from_url(query)
            if bibcode:
                if self._library and self._library.has_bibcode(bibcode):
                    self._set_status(f"[yellow]{bibcode} already in library.[/yellow]")
                    return
                self._set_status(f"Importing {bibcode}…")
                self.run_worker(self._fetch_and_add(bibcode, None), exclusive=True)
                return
            tab_data = tabs_state.make_tab(query, limit=limit)
            self._tab_states.append(tab_data)
            await self._create_ads_tab(tab_data, fetch=True)

        initial = ""
        if self._active_pane_id() == "pane-manuscript":
            ms_view = self.query_one(ManuscriptView)
            if ms_view._highlighted_state == "missing" and ms_view._highlighted_key:
                initial = ms_view._highlighted_key
        self.push_screen(AdsSearchModal(initial=initial), handle)

    async def action_close_tab(self) -> None:
        pane_id = self._active_pane_id()
        if not pane_id or pane_id in ("pane-library", "pane-manuscript"):
            return
        tab_id = pane_id.removeprefix("pane-")
        self._ads_views.pop(tab_id, None)
        self._tab_states = [t for t in self._tab_states if t["id"] != tab_id]
        tabs_state.save(self._tab_states, self._ms_root)
        await self.query_one(TabbedContent).remove_pane(pane_id)

    async def action_refresh_tab(self) -> None:
        ads_view = self._active_ads_view()
        if ads_view is None:
            return
        tab_data = next((t for t in self._tab_states if t["id"] == ads_view.tab_id), None)
        if tab_data:
            await self._do_refresh(ads_view, tab_data)

    async def action_references(self) -> None:
        await self._open_linked_tab("references")

    async def action_citations(self) -> None:
        await self._open_linked_tab("citations")

    async def _open_linked_tab(self, kind: str) -> None:
        """Open a new ADS tab listing the references of / citations to the
        highlighted paper, using ADS's references()/citations() operators."""
        from .. import pdf as _pdf
        if not get_token():
            self.push_screen(ConfigModal())
            self._set_status("[yellow]Set your ADS token first.[/yellow]")
            return
        ads_view = self._active_ads_view()
        if ads_view is not None:
            a = ads_view._selected_article
            if a is None:
                self._set_status("[yellow]Select an article first.[/yellow]")
                return
            bibcode = a.bibcode
            surname = (a.author[0].split(",")[0] if a.author else "").replace(" ", "")
            label_key = f"{surname}{a.year or ''}" or bibcode
        else:
            entry = self.query_one(LibraryView)._highlighted_entry
            if entry is None:
                self._set_status("[yellow]Select a paper first.[/yellow]")
                return
            bibcode = _pdf._bibcode_from_adsurl(entry.adsurl)
            if not bibcode:
                self._set_status(f"[yellow]No ADS URL for {entry.key}[/yellow]")
                return
            label_key = entry.short_key or entry.key
        prefix, arrow = ("refs", "←") if kind == "references" else ("cites", "→")
        query = f'{kind}(bibcode:"{bibcode}")'
        tab_data = tabs_state.make_tab(query, label=f"{prefix}{arrow}{label_key}")
        self._tab_states.append(tab_data)
        await self._create_ads_tab(tab_data, fetch=True)

    def action_toggle_manuscript(self) -> None:
        """Toggle manuscript-db membership for selected (or highlighted) entries."""
        if self._library is None or self._library.manuscript is None:
            return
        if self._active_pane_id() == "pane-manuscript":
            ms_view = self.query_one(ManuscriptView)
            key, state = ms_view._highlighted_key, ms_view._highlighted_state
            entry = ms_view._highlighted_entry
            if not key:
                return
            if state == "library" and entry is not None:
                self._library.add_to_manuscript(entry.key)
                self._set_status(f"[green]Added {entry.key} to manuscript db[/green]")
            elif state in ("ok", "uncited") and entry is not None:
                rescued = not self._library.in_personal(entry.key)
                self._library.remove_from_manuscript(entry.key)
                note = "  [dim](copied to personal library)[/dim]" if rescued else ""
                self._set_status(f"Removed {entry.key} from manuscript db{note}")
            elif state == "ambiguous":
                self._set_status(f"[magenta]{key} is ambiguous — lengthen it in the .tex[/magenta]")
                return
            else:
                self._set_status(f"[yellow]{key} not found anywhere — S to search ADS[/yellow]")
                return
            # Move the cursor to the next row of the same state (else the
            # previous one), so repeated m presses walk through candidates
            # instead of following the toggled row to its new sort position.
            rows = ms_view._rows
            idx = next((i for i, r in enumerate(rows) if r[0] == key), None)
            if idx is not None:
                same = [(i, r[0]) for i, r in enumerate(rows)
                        if r[1] == state and r[0] != key]
                nxt = next((k for i, k in same if i > idx),
                           same[-1][1] if same else None)
                if nxt is not None:
                    ms_view._highlighted_key = nxt
            self._refresh_manuscript_view()
            try:
                self.query_one(LibraryView)._refresh_table()
            except Exception:
                pass
            return
        lib_view = self.query_one(LibraryView)
        keys = lib_view.get_selected_keys()
        if not keys and lib_view._highlighted_entry is not None:
            keys = [lib_view._highlighted_entry.key]
        if not keys:
            return
        # If any selected entry is missing from the manuscript, add all;
        # otherwise (all present) remove all.
        missing = [k for k in keys if not self._library.in_manuscript(k)]
        t = lib_view.query_one("#lib-table", DataTable)
        if missing:
            for k in missing:
                self._library.add_to_manuscript(k)
            val, n = "◆", len(missing)
            self._set_status(f"[green]Added {n} paper(s) to manuscript db[/green]")
            changed = missing
        else:
            for k in keys:
                self._library.remove_from_manuscript(k)
            val, n = "", len(keys)
            self._set_status(f"Removed {n} paper(s) from manuscript db")
            changed = keys
        for k in changed:
            try:
                t.update_cell(k, "lib", val)
            except Exception:
                lib_view._refresh_table()
                break
        self._refresh_manuscript_view()
        self.refresh_bindings()

    def action_toggle_ms_only(self) -> None:
        lib_view = self.query_one(LibraryView)
        on = lib_view.toggle_ms_only()
        name = self._ms_root.name if self._ms_root else "manuscript"
        self._set_status(f"Showing [cyan]{name}[/cyan] entries only  [dim](M to show all)[/dim]"
                         if on else "Showing all entries")

    def _copy_text(self, text: str) -> bool:
        """Copy text to the system clipboard: pbcopy on macOS (reliable in
        any terminal), else Textual's OSC 52 escape (terminal-dependent)."""
        import subprocess
        import sys
        if sys.platform == "darwin":
            try:
                subprocess.run(["pbcopy"], input=text.encode(), check=True)
                return True
            except Exception:
                pass
        try:
            self.copy_to_clipboard(text)
            return True
        except Exception:
            return False

    def action_copy_key(self, full: bool = False) -> None:
        """Copy the highlighted (or selected) cite key(s) to the clipboard.

        y copies the shortest unambiguous form; Y copies full keys.
        """
        def pick(e: Entry) -> str:
            return e.key if full else (e.short_key or e.key)

        pane_id = self._active_pane_id()
        ads_view = self._active_ads_view()
        text = None
        if pane_id == "pane-library":
            lib_view = self.query_one(LibraryView)
            keys = lib_view.get_selected_keys()
            if keys and self._library:
                entries = [self._library.get(k) for k in keys]
                text = ", ".join(pick(e) for e in entries if e)
            elif lib_view._highlighted_entry is not None:
                text = pick(lib_view._highlighted_entry)
        elif pane_id == "pane-manuscript":
            ms_view = self.query_one(ManuscriptView)
            e = ms_view._highlighted_entry
            if e is not None:
                text = pick(e)
            else:
                text = ms_view._highlighted_key
        elif ads_view is not None:
            def _key_for(bibcode: str) -> str:
                entry = self._library.get_by_bibcode(bibcode) if self._library else None
                return pick(entry) if entry else bibcode
            if ads_view._selected_bibcodes:
                text = ", ".join(_key_for(a.bibcode) for a in ads_view._articles
                                 if a.bibcode in ads_view._selected_bibcodes)
            else:
                a = ads_view._selected_article
                if a is not None:
                    text = _key_for(a.bibcode)
        if not text:
            self._set_status("[yellow]Nothing to copy.[/yellow]")
            return
        if self._copy_text(text):
            self._set_status(f"[green]Copied:[/green] {text}")
        else:
            self._set_status("[red]Clipboard copy failed — terminal may not support OSC 52[/red]")

    def action_more_results(self) -> None:
        self._step_tab_limit(+1)

    def action_fewer_results(self) -> None:
        self._step_tab_limit(-1)

    def _step_tab_limit(self, direction: int) -> None:
        ads_view = self._active_ads_view()
        if ads_view is None:
            return
        tab_data = next((t for t in self._tab_states if t["id"] == ads_view.tab_id), None)
        if tab_data is None:
            return
        new_limit = tabs_state.step_limit(tab_data, direction)
        tabs_state.save(self._tab_states, self._ms_root)
        n = len(ads_view._articles)
        self._set_status(_ads_tab_status(ads_view.query, n, new_limit)
                         + "  [dim]· r to reload[/dim]")

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

    async def _fetch_and_add(self, bibcode: str, ads_view: "AdsView | None") -> None:
        import asyncio
        import shutil
        try:
            from .. import ads_client, pdf as _pdf
            data = await asyncio.to_thread(ads_client.fetch_bibtex, bibcode)
            if data is None:
                self._set_status(f"[red]Could not fetch {bibcode}[/red]")
                return
            entry = self._library.save_entry(data)
            bibcode_pdf = _pdf.cache_path(bibcode)
            if bibcode_pdf.exists() and not _pdf.cache_path(entry.key).exists():
                shutil.copy2(str(bibcode_pdf), str(_pdf.cache_path(entry.key)))
            self._reload_library()
            if ads_view is not None:
                ads_view.update_lib_status(self._library)
            self.query_one(DetailPanel).show_entry(self._library.get(entry.key))
            self.refresh_bindings()
            ads_client.refresh_quota()
            self._set_status(f"[green]Added {entry.short_key or entry.key}[/green]{_quota_str()}")
        except Exception as exc:
            self._set_status(f"[red]{exc}[/red]")

    async def _fetch_and_add_batch(self, bibcodes: list[str], ads_view: AdsView) -> None:
        import asyncio
        import shutil
        from .. import ads_client, pdf as _pdf
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
                bibcode_pdf = _pdf.cache_path(bibcode)
                if bibcode_pdf.exists() and not _pdf.cache_path(entry.key).exists():
                    shutil.copy2(str(bibcode_pdf), str(_pdf.cache_path(entry.key)))
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
        if self._poll_cancel is not None:
            self._cancel_poll()
            self.query_one(DetailPanel).set_pdf_status("[dim]Cancelled[/dim]")
            self._set_status("[dim]Browser download cancelled[/dim]")
            return
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
            adsurl = f"https://ui.adsabs.harvard.edu/abs/{article.bibcode}"
        else:
            entry = self.query_one(LibraryView)._highlighted_entry
            if entry is None:
                return
            eprint, doi, key, adsurl = entry.eprint, entry.doi, entry.key, entry.adsurl
        if not eprint and not doi:
            self._set_status(f"[yellow]No arXiv ID or DOI for {key}[/yellow]")
            return
        self._cancel_poll()
        self._set_status(f"Downloading {key}…")
        self.run_worker(self._do_download_pdf(key, eprint, doi, adsurl, ads_view), exclusive=True)

    async def _do_download_pdf(self, key: str, eprint: str, doi: str,
                                adsurl: str, ads_view: "AdsView | None",
                                source: str = "auto") -> None:
        import asyncio
        from .. import pdf
        detail = self.query_one(DetailPanel)
        path = await asyncio.to_thread(
            pdf.fetch, key, eprint=eprint, doi=doi, adsurl=adsurl, source=source, force=True
        )
        if path is None:
            if pdf.browser_open_url(doi=doi, adsurl=adsurl):
                self._set_status(f"[red]No auto PDF for {key}[/red]  [dim]→ press B or click browser ↓[/dim]")
                detail.set_pdf_status("[red]✗ not found — try browser ↓[/red]")
            else:
                self._set_status(f"[red]No PDF found for {key}[/red]")
                detail.set_pdf_status("[red]✗ no PDF available[/red]")
        else:
            sz = path.stat().st_size // 1024
            self._set_status(f"[green]Downloaded {key}  ({sz} KB)[/green]")
            detail.set_pdf_status(f"[green]✓ {sz} KB[/green]")
            if ads_view is not None:
                ads_view.refresh_pdf_status()
            else:
                self.query_one(LibraryView).refresh_pdf_status()
            self.refresh_bindings()

    def action_open_pdf(self) -> None:
        import subprocess
        import sys
        from .. import pdf, ads_client as _ac
        ads_view = self._active_ads_view()
        if ads_view is not None:
            bcs = {bc for bc in ads_view._selected_bibcodes if pdf.is_cached(bc)}
            a = ads_view._selected_article
            if a and pdf.is_cached(a.bibcode):
                bcs.add(a.bibcode)
            if bcs:
                paths = [str(pdf.cache_path(bc)) for bc in bcs]
                if sys.platform == "darwin":
                    subprocess.run(["open"] + paths, check=False)
                elif sys.platform.startswith("linux"):
                    for p in paths:
                        subprocess.run(["xdg-open", p], check=False)
                else:
                    for p in paths:
                        subprocess.run(["start", p], shell=True, check=False)
                self._set_status(f"[green]Opened {len(paths)} PDF(s)[/green]")
                return
        if ads_view is not None:
            article = ads_view._selected_article
            if article is None:
                self._set_status("[yellow]Select an article first.[/yellow]")
                return
            eprint = _ac.arxiv_id_from_article(article)
            doi = (article.doi or [""])[0]
            key = article.bibcode
            adsurl = f"https://ui.adsabs.harvard.edu/abs/{article.bibcode}"
        else:
            lib_view = self.query_one(LibraryView)
            if lib_view._selected_keys:
                paths = [str(pdf.cache_path(k)) for k in lib_view._selected_keys if pdf.is_cached(k)]
                if paths:
                    if sys.platform == "darwin":
                        subprocess.run(["open"] + paths, check=False)
                    elif sys.platform.startswith("linux"):
                        for p in paths:
                            subprocess.run(["xdg-open", p], check=False)
                    else:
                        for p in paths:
                            subprocess.run(["start", p], shell=True, check=False)
                    self._set_status(f"[green]Opened {len(paths)} PDF(s)[/green]")
                return
            entry = lib_view._highlighted_entry
            if entry is None:
                self._set_status("[yellow]Select a paper first.[/yellow]")
                return
            eprint, doi, key, adsurl = entry.eprint, entry.doi, entry.key, entry.adsurl
        if not eprint and not doi:
            self._set_status(f"[yellow]No arXiv ID or DOI for {key}[/yellow]")
            return
        if pdf.is_cached(key):
            self._set_status(f"Opening {key}…")
        elif doi:
            self._set_status(f"Fetching {key} via ADS OA resolver…")
        else:
            self._set_status(f"Fetching {key} from arXiv:{eprint}…")
        self.run_worker(self._do_open_pdf(key, eprint, doi, adsurl, ads_view), exclusive=True)

    async def _do_open_pdf(self, key: str, eprint: str, doi: str,
                            adsurl: str, ads_view: "AdsView | None") -> None:
        import asyncio
        from .. import pdf
        detail = self.query_one(DetailPanel)
        opened = await asyncio.to_thread(
            pdf.open_pdf, key, eprint=eprint, doi=doi, adsurl=adsurl
        )
        if not opened:
            if pdf.browser_open_url(doi=doi, adsurl=adsurl):
                self._set_status(f"[red]No auto PDF for {key}[/red]  [dim]→ press B or click browser ↓[/dim]")
                detail.set_pdf_status("[red]✗ not found — try browser ↓[/red]")
            else:
                self._set_status(f"[red]No open-access PDF found for {key}[/red]")
                detail.set_pdf_status("[red]✗ no PDF available[/red]")
        else:
            self._set_status(f"[green]Opened {key}[/green]")
            detail.set_pdf_status("[green]✓ opened[/green]")
            if ads_view is not None:
                ads_view.refresh_pdf_status()
            else:
                self.query_one(LibraryView).refresh_pdf_status()
            self.refresh_bindings()

    def action_browser_pdf(self) -> None:
        from .. import pdf, ads_client as _ac
        ads_view = self._active_ads_view()
        if ads_view is not None:
            article = ads_view._selected_article
            if article is None:
                return
            doi = (article.doi or [""])[0]
            key = article.bibcode
            adsurl = f"https://ui.adsabs.harvard.edu/abs/{article.bibcode}"
            eprint = _ac.arxiv_id_from_article(article)
        else:
            entry = self.query_one(LibraryView)._highlighted_entry
            if entry is None:
                return
            doi, key, adsurl, eprint = entry.doi, entry.key, entry.adsurl, entry.eprint
        if not pdf.browser_open_url(doi=doi, adsurl=adsurl, eprint=eprint):
            self._set_status(f"[yellow]No DOI, ADS URL, or arXiv ID for {key}[/yellow]")
            return
        self._cancel_poll()
        self.run_worker(self._do_browser_pdf(key, doi, adsurl, eprint, ads_view), exclusive=True)

    async def _do_browser_pdf(self, key: str, doi: str, adsurl: str, eprint: str | None,
                               ads_view: "AdsView | None") -> None:
        import asyncio
        from .. import pdf
        self._set_status(f"Resolving browser source for {key}…")
        url = await asyncio.to_thread(
            lambda: pdf.browser_resolve_url(doi=doi, adsurl=adsurl, eprint=eprint)
        )
        if url is None:
            self._set_status(f"[red]No browser source found for {key}[/red]")
            return
        before = pdf.downloads_snapshot()
        pdf.browser_open(url)
        cancel = threading.Event()
        self._poll_cancel = cancel
        detail = self.query_one(DetailPanel)
        detail.set_pdf_status("[yellow]⏳ Waiting for download…  [dim](X to cancel)[/dim][/yellow]")
        self._set_status(f"[yellow]Browser opened — waiting for PDF in ~/Downloads (60s)…[/yellow]")
        path = await asyncio.to_thread(pdf.poll_downloads, key, before, 60, cancel)
        self._poll_cancel = None
        if path is None:
            if cancel.is_set():
                detail.set_pdf_status("[dim]Cancelled[/dim]")
                self._set_status("[dim]Browser download cancelled[/dim]")
            else:
                detail.set_pdf_status("[red]✗ Timed out (60s)[/red]")
                self._set_status(f"[red]No PDF appeared in ~/Downloads within 60s[/red]")
        else:
            sz = path.stat().st_size // 1024
            detail.set_pdf_status(f"[green]✓ {sz} KB[/green]")
            self._set_status(f"[green]Downloaded {key}  ({sz} KB)[/green]")
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
            self._set_status("[yellow]UAT not cached — run: astrobib uat update[/yellow]")
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

        # Actions only valid on ADS tabs (refresh_tab is footer-visible:
        # greyed on the library and manuscript tabs)
        if action == "refresh_tab":
            return True if ads_view is not None else None
        if action in ("close_tab", "more_results", "fewer_results"):
            return ads_view is not None

        # Filter on library, edit query on ADS tabs — valid on both
        if action == "filter":
            return True if (on_library or ads_view) else None
        if action == "export_selected":
            return on_library

        if action == "toggle_select":
            if on_library:
                return True
            if ads_view:
                return ads_view._selected_article is not None
            return False

        if action in ("toggle_manuscript", "toggle_ms_only"):
            has_ms = bool(self._library and self._library.manuscript)
            on_manuscript = pane_id == "pane-manuscript"
            if action == "toggle_ms_only":
                return on_library and has_ms
            if on_library and has_ms:
                lib_view = self.query_one(LibraryView)
                return bool(lib_view._selected_keys or lib_view._highlighted_entry)
            if on_manuscript and has_ms:
                return self.query_one(ManuscriptView)._highlighted_key is not None
            return False

        if action in ("references", "citations"):
            if on_library:
                entry = self.query_one(LibraryView)._highlighted_entry
                return entry is not None and bool(entry.adsurl)
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
            if self._poll_cancel is not None:
                return True
            if on_library:
                entry = self.query_one(LibraryView)._highlighted_entry
                return entry is not None and _pdf.is_cached(entry.key)
            if ads_view and ads_view._selected_article:
                a = ads_view._selected_article
                return (self._library is not None and self._library.has_bibcode(a.bibcode)
                        and _pdf.is_cached(a.bibcode))
            return False

        if action == "open_pdf":
            if on_library:
                lib_view = self.query_one(LibraryView)
                if lib_view._selected_keys:
                    return all(_pdf.is_cached(k) for k in lib_view._selected_keys) or None
                entry = lib_view._highlighted_entry
                return (entry is not None and _pdf.is_cached(entry.key)) or None
            if ads_view:
                a = ads_view._selected_article
                cursor_cached = a is not None and _pdf.is_cached(a.bibcode)
                selected_cached = any(_pdf.is_cached(bc) for bc in ads_view._selected_bibcodes)
                return (cursor_cached or selected_cached) or None
            return None

        if action == "download_pdf":
            if on_library:
                entry = self.query_one(LibraryView)._highlighted_entry
                return (entry is not None and bool(entry.eprint or entry.doi)) or None
            if ads_view:
                if ads_view._selected_bibcodes:
                    return None
                a = ads_view._selected_article
                if a is None or not (self._library and self._library.has_bibcode(a.bibcode)):
                    return None
                return bool(a.identifier or (a.doi and a.doi[0])) or None
            return None

        if action == "browser_pdf":
            if on_library:
                entry = self.query_one(LibraryView)._highlighted_entry
                return entry is not None and bool(entry.doi or entry.adsurl)
            if ads_view:
                a = ads_view._selected_article
                if a is None or not (self._library and self._library.has_bibcode(a.bibcode)):
                    return False
                return bool((a.doi and a.doi[0]) or a.bibcode)
            return False

        if action == "add_paper":
            if on_library:
                return None
            if ads_view is not None:
                if ads_view._selected_bibcodes:
                    return any(
                        not (self._library and self._library.has_bibcode(bc))
                        for bc in ads_view._selected_bibcodes
                    ) or None
                article = ads_view._selected_article
                if article and self._library:
                    return (self._library.get_by_bibcode(article.bibcode) is None) or None
            return True

        if action == "remove_paper":
            if on_library:
                lib_view = self.query_one(LibraryView)
                return (lib_view._highlighted_entry is not None) or None
            if ads_view is not None:
                article = ads_view._selected_article
                if article and self._library:
                    return (self._library.get_by_bibcode(article.bibcode) is not None) or None
            return None

        return True

    def _set_status(self, msg: str) -> None:
        self.query_one("#status-bar", Static).update(msg)


# ── Helpers ───────────────────────────────────────────────────────────────────


def _ads_tab_status(query: str, n: int, limit: int) -> str:
    return f"[bold]{query}[/bold]  {n} results  [dim]n={limit} (+ -)[/dim]"


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


