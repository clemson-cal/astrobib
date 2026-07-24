from __future__ import annotations

import importlib.resources

from textual.app import ComposeResult
from textual.binding import Binding
from textual.screen import ModalScreen
from textual.widgets import Footer, Markdown


def _load_help() -> str:
    return importlib.resources.files("astrobib").joinpath("help.md").read_text()


class HelpScreen(ModalScreen):
    DEFAULT_CSS = """
    HelpScreen {
        align: center middle;
    }
    HelpScreen Markdown {
        width: 90;
        height: 90%;
        max-height: 90vh;
        border: round $primary;
        background: $surface;
        padding: 1 2;
        overflow-y: auto;
    }
    """

    BINDINGS = [
        Binding("q,escape,question_mark", "dismiss", "Close", show=True),
    ]

    def compose(self) -> ComposeResult:
        yield Markdown(_load_help())
        yield Footer()
