"""Modal file picker for importing a PDF from the filesystem."""
from __future__ import annotations

from pathlib import Path
from typing import Iterable, Optional

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import DirectoryTree, Static


class PdfDirectoryTree(DirectoryTree):
    """Directory tree showing only directories and PDF files."""

    def filter_paths(self, paths: Iterable[Path]) -> Iterable[Path]:
        return [
            p for p in paths
            if not p.name.startswith(".")
            and (p.is_dir() or p.suffix.lower() == ".pdf")
        ]


class FilePickScreen(ModalScreen[Optional[Path]]):
    """Pick a PDF file; dismisses with the chosen Path or None."""

    BINDINGS = [
        Binding("escape", "cancel", "Cancel"),
        Binding("backspace", "up_dir", "Parent dir"),
    ]

    DEFAULT_CSS = """
    FilePickScreen {
        align: center middle;
    }
    FilePickScreen #pick-box {
        width: 70%;
        max-width: 100;
        height: 70%;
        background: $surface;
        border: round $primary;
        padding: 0 1;
    }
    FilePickScreen #pick-title {
        height: 1;
        color: $text-accent;
        text-style: bold;
    }
    FilePickScreen #pick-root {
        height: 1;
        color: $text-muted;
    }
    FilePickScreen PdfDirectoryTree {
        height: 1fr;
    }
    FilePickScreen #pick-hint {
        height: 1;
        color: $text-muted;
    }
    """

    def __init__(self, start: Path | None = None) -> None:
        super().__init__()
        downloads = Path.home() / "Downloads"
        self._root = start or (downloads if downloads.is_dir() else Path.home())

    def compose(self) -> ComposeResult:
        with Vertical(id="pick-box"):
            yield Static("Pick a PDF", id="pick-title")
            yield Static(str(self._root), id="pick-root")
            yield PdfDirectoryTree(self._root)
            yield Static("⏎ select   ⌫ parent dir   esc cancel", id="pick-hint")

    def on_mount(self) -> None:
        self.query_one(PdfDirectoryTree).focus()

    def on_directory_tree_file_selected(
        self, event: DirectoryTree.FileSelected
    ) -> None:
        event.stop()
        self.dismiss(Path(event.path))

    def action_cancel(self) -> None:
        self.dismiss(None)

    def action_up_dir(self) -> None:
        parent = self._root.parent
        if parent == self._root:
            return
        self._root = parent
        self.query_one("#pick-root", Static).update(str(parent))
        tree = self.query_one(PdfDirectoryTree)
        tree.path = parent
        tree.reload()
