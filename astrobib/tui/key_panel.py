"""Key/help panel that dims unavailable actions.

Textual's stock BindingsTable (behind the ctrl+p "Show keys and help
panel" command) receives each binding's enabled flag but ignores it when
rendering, so every action looks available. These subclasses render
disabled bindings dim, matching the footer. The table rendering mirrors
textual.widgets._key_panel.BindingsTable (Textual 8.x) with the enabled
flag applied.
"""
from __future__ import annotations

from collections import defaultdict
from itertools import groupby
from operator import itemgetter

from rich import box
from rich.table import Table
from rich.text import Text

from textual.app import ComposeResult
from textual.widgets import HelpPanel, KeyPanel, Markdown
from textual.widgets._key_panel import BindingsTable


class DimBindingsTable(BindingsTable):
    def render_bindings_table(self) -> Table:
        bindings = self.screen.active_bindings.values()

        key_style = self.get_component_rich_style("bindings-table--key")
        divider_transparent = (
            self.get_component_styles("bindings-table--divider").color.a == 0
        )
        table = Table(
            padding=(0, 0),
            show_header=False,
            box=box.SIMPLE if divider_transparent else box.HORIZONTALS,
            border_style=self.get_component_rich_style("bindings-table--divider"),
        )
        table.add_column("", justify="right")

        header_style = self.get_component_rich_style("bindings-table--header")
        description_style = self.get_component_rich_style("bindings-table--description")
        get_key_display = self.app.get_key_display
        previous_namespace: object = None
        for namespace, _bindings in groupby(bindings, key=itemgetter(0)):
            table_bindings = list(_bindings)
            if not table_bindings:
                continue

            if namespace.BINDING_GROUP_TITLE:
                title = Text(namespace.BINDING_GROUP_TITLE, end="")
                title.stylize(header_style)
                table.add_row("", title)

            action_to_bindings = defaultdict(list)
            for _, binding, enabled, tooltip in table_bindings:
                if not binding.system:
                    action_to_bindings[binding.action].append(
                        (binding, enabled, tooltip)
                    )

            for multi_bindings in action_to_bindings.values():
                binding, enabled, _tooltip = multi_bindings[0]
                keys_display = " ".join(
                    dict.fromkeys(get_key_display(b) for b, _, _ in multi_bindings)
                )
                key_text = Text(keys_display, style=key_style)
                desc_text = Text.from_markup(
                    binding.description, end="", style=description_style
                )
                if binding.tooltip:
                    if binding.description:
                        desc_text.append(" ")
                    desc_text.append(binding.tooltip, "dim")
                if not enabled:
                    key_text.stylize("dim")
                    desc_text.stylize("dim")
                table.add_row(key_text, desc_text)

            if namespace != previous_namespace:
                table.add_section()
            previous_namespace = namespace

        return table


class DimKeyPanel(KeyPanel):
    def compose(self) -> ComposeResult:
        yield DimBindingsTable(shrink=True, expand=False)


class DimHelpPanel(HelpPanel):
    def compose(self) -> ComposeResult:
        yield Markdown(id="widget-help")
        yield DimKeyPanel(id="keys-help")
