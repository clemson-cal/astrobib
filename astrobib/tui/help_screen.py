from __future__ import annotations

import importlib.resources

from textual.app import ComposeResult
from textual.binding import Binding
from textual.screen import ModalScreen
from textual.widgets import Footer, Markdown


_HELP_FALLBACK = """\
# astrobib help

The bundled help file (`astrobib/help.md`) is missing from this
installation. It is a copy of the project README; see
<https://github.com/jzrake/astrobib> for the full documentation.

Press `q` or `escape` to close this screen.
"""


def _load_help() -> str:
    try:
        return importlib.resources.files("astrobib").joinpath("help.md").read_text()
    except OSError:
        return _HELP_FALLBACK


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
