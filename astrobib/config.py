import os
import tomllib
from dataclasses import dataclass, field
from pathlib import Path


CONFIG_FILE = Path.home() / ".config" / "astrobib" / "config.toml"
DEFAULT_DB_DIR = Path.home() / ".local" / "share" / "astrobib" / "databases"
UAT_CACHE = Path.home() / ".cache" / "astrobib" / "uat.json"
PDF_CACHE_DIR = Path.home() / ".cache" / "astrobib" / "pdfs"


@dataclass
class Config:
    databases: dict[str, Path]      # name -> resolved path
    default_database: str           # name of the default write target
    ads_token: str
    pdf_cache_dir: Path

    @property
    def default_db_path(self) -> Path:
        return self.databases[self.default_database]

    def db_path(self, name: str | None) -> Path:
        """Resolve a database name (or None → default) to a path."""
        name = name or self.default_database
        if name not in self.databases:
            known = ", ".join(self.databases)
            raise RuntimeError(f"Unknown database '{name}'. Configured: {known}")
        return self.databases[name]


_config: Config | None = None


def get_config() -> Config:
    global _config
    if _config is not None:
        return _config
    _config = _load_config()
    return _config


def _load_config() -> Config:
    ads_token = os.environ.get("ADS_API_TOKEN", "")
    pdf_cache_dir = PDF_CACHE_DIR
    pdf_cache_dir.mkdir(parents=True, exist_ok=True)

    databases: dict[str, Path] = {}
    default_database = ""

    # Env var override for a single database
    if env := os.environ.get("ASTROBIB_DB"):
        p = Path(env).expanduser().resolve()
        databases["default"] = p
        default_database = "default"

    # Walk up from cwd — supports working directly inside a bib database repo
    if not databases:
        path = Path.cwd().resolve()
        while path != path.parent:
            if (path / "bib").is_dir() and (path / ".git").exists():
                databases["default"] = path
                default_database = "default"
                break
            path = path.parent

    # Config file
    if CONFIG_FILE.exists():
        with open(CONFIG_FILE, "rb") as f:
            data = tomllib.load(f)

        ads_token = data.get("ads_token", ads_token)

        # Legacy single-database form
        if "database" in data and not databases:
            p = Path(data["database"]).expanduser().resolve()
            databases["default"] = p
            default_database = "default"

        # Named databases
        if "databases" in data:
            for name, db_data in data["databases"].items():
                p = Path(db_data["path"]).expanduser().resolve()
                databases[name] = p
            if not default_database:
                default_database = data.get("default_database", next(iter(databases)))

    if not databases:
        raise RuntimeError(
            "No bib database configured.\n"
            "Clone an existing one:  astrobib db clone <git-url>\n"
            "Create a new one:       astrobib db init <path>\n"
            "Or set:                 ASTROBIB_DB=/path/to/database"
        )

    return Config(
        databases=databases,
        default_database=default_database,
        ads_token=ads_token,
        pdf_cache_dir=pdf_cache_dir,
    )


def save_database(name: str, db_path: Path) -> None:
    """Add or update a named database entry in the config file."""
    CONFIG_FILE.parent.mkdir(parents=True, exist_ok=True)

    raw: dict = {}
    if CONFIG_FILE.exists():
        with open(CONFIG_FILE, "rb") as f:
            raw = dict(tomllib.load(f))

    # Migrate legacy single-database entry
    if "database" in raw:
        old_path = raw.pop("database")
        raw.setdefault("databases", {})
        raw["databases"].setdefault("default", {"path": old_path})
        raw.setdefault("default_database", "default")

    raw.setdefault("databases", {})
    raw["databases"][name] = {"path": str(db_path)}

    if "default_database" not in raw:
        raw["default_database"] = name

    _write_toml(CONFIG_FILE, raw)

    # Invalidate cached config
    global _config
    _config = None


def _write_toml(path: Path, data: dict) -> None:
    lines: list[str] = []
    for key, value in data.items():
        if key == "databases":
            continue
        if isinstance(value, str):
            lines.append(f'{key} = "{value}"')
        elif isinstance(value, bool):
            lines.append(f"{key} = {'true' if value else 'false'}")
        elif isinstance(value, (int, float)):
            lines.append(f"{key} = {value}")
    if "databases" in data:
        for db_name, db_data in data["databases"].items():
            lines.append(f"\n[databases.{db_name}]")
            for k, v in db_data.items():
                lines.append(f'{k} = "{v}"')
    path.write_text("\n".join(lines) + "\n")
