from __future__ import annotations

from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.screen import Screen
from textual.widgets import Footer, Header, Static, Tree
from textual.widgets.tree import TreeNode

from ..uat import UAT, Concept


class ConceptDetail(Static):
    DEFAULT_CSS = """
    ConceptDetail {
        height: 100%;
        padding: 1 2;
        border-left: solid $panel-lighten-1;
        overflow-y: auto;
    }
    """

    def show(self, concept: Concept | None, uat: UAT) -> None:
        if concept is None:
            self.update("[dim]Select a concept to see details.[/dim]")
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


class UATBrowserScreen(Screen):
    """UAT hierarchy browser — usable as a pushed screen or standalone."""

    DEFAULT_CSS = """
    UATBrowserScreen { layout: vertical; }
    #uat-body { layout: horizontal; height: 1fr; }
    #uat-tree { width: 50; border-right: solid $panel-lighten-1; }
    #uat-detail { width: 1fr; }
    """

    BINDINGS = [
        Binding("q,escape", "dismiss_or_quit", "Back", show=True),
    ]

    def __init__(self, uat: UAT):
        super().__init__()
        self._uat = uat

    def compose(self) -> ComposeResult:
        yield Header(show_clock=False)
        with Horizontal(id="uat-body"):
            yield Tree("UAT concepts", id="uat-tree")
            yield ConceptDetail(id="uat-detail")
        yield Footer()

    def on_mount(self) -> None:
        self.title = "UAT Browser"
        tree = self.query_one(Tree)
        tree.root.expand()
        for concept in sorted(self._uat.top_level(), key=lambda c: c.label):
            _add_node(tree.root, concept)
        self.query_one(ConceptDetail).show(None, self._uat)

    @on(Tree.NodeExpanded)
    def on_node_expanded(self, event: Tree.NodeExpanded) -> None:
        node = event.node
        concept: Concept | None = node.data
        if concept is None or node.children:
            return
        for child in sorted(self._uat.children(concept.uid), key=lambda c: c.label):
            _add_node(node, child)

    @on(Tree.NodeSelected)
    def on_node_selected(self, event: Tree.NodeSelected) -> None:
        concept: Concept | None = event.node.data
        self.query_one(ConceptDetail).show(concept, self._uat)

    def action_dismiss_or_quit(self) -> None:
        if self.app.screen_stack and len(self.app.screen_stack) > 1:
            self.app.pop_screen()
        else:
            self.app.exit()


class UATBrowserApp(App):
    """Thin wrapper so UATBrowserScreen can run as a standalone app."""

    def __init__(self, uat: UAT):
        super().__init__()
        self._uat = uat

    def on_mount(self) -> None:
        self.push_screen(UATBrowserScreen(self._uat))


def _add_node(parent: TreeNode, concept: Concept) -> TreeNode:
    return parent.add(
        concept.label,
        data=concept,
        allow_expand=bool(concept.narrower),
    )
