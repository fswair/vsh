"""Validate the built docs without importing VSH, compiling Rust or accessing the network."""

from __future__ import annotations

import ast
import json
import re
import textwrap
import tomllib
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urljoin, urlsplit

ROOT = Path(__file__).resolve().parents[1]


class Page(HTMLParser):
    """Collect static links, fragment targets and copy-source keys."""

    def __init__(self, source: str) -> None:
        super().__init__()
        self.links: list[str] = []
        self.ids: set[str] = set()
        self.copy_keys: list[str | None] = []
        self.feed(source)

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        identifier = values.get("id")
        if identifier:
            self.ids.add(identifier)
        for key in ("href", "src"):
            if value := values.get(key):
                self.links.append(value)
        if tag == "button" and "vsh-copy-markdown" in (values.get("class") or "").split():
            self.copy_keys.append(values.get("data-source-key"))


def main() -> None:
    config = tomllib.loads((ROOT / "zensical.toml").read_text(encoding="utf-8"))["project"]
    site = ROOT / config["site_dir"]
    docs = ROOT / config["docs_dir"]
    site_url = config["site_url"].rstrip("/") + "/"
    base = urlsplit(site_url)
    prefix = base.path
    corpus = json.loads((docs / "assets/markdown.json").read_text(encoding="utf-8"))
    parsed = {path: Page(path.read_text(encoding="utf-8")) for path in site.rglob("*.html")}
    errors: list[str] = []
    checked_links = 0
    for source, page in parsed.items():
        route = source.relative_to(site).as_posix().removesuffix("index.html")
        for link in page.links:
            target = urlsplit(urljoin(site_url + route, link))
            if target.scheme not in {"http", "https"} or target.netloc != base.netloc:
                continue
            if not target.path.startswith(prefix):
                errors.append(f"{route}: local link escapes site prefix: {link}")
                continue
            destination = site / unquote(target.path[len(prefix) :])
            if destination.is_dir():
                destination /= "index.html"
            checked_links += 1
            if not destination.is_file():
                errors.append(f"{route}: missing local target: {link}")
            elif (
                target.fragment
                and destination in parsed
                and unquote(target.fragment) not in parsed[destination].ids
            ):
                errors.append(f"{route}: missing fragment: {link}")
    for key in corpus:
        destination = site / key / "index.html"
        if destination not in parsed or parsed[destination].copy_keys != [key]:
            errors.append(f"{key}: missing or ambiguous Copy as Markdown source mapping")
    for name in ("llms.txt", "llms-full.txt", "assets/markdown.json"):
        if (docs / name).read_bytes() != (site / name).read_bytes():
            errors.append(f"{name}: deployed bundle differs from generated source")
    snippets = 0
    for source in docs.rglob("*.md"):
        text = source.read_text(encoding="utf-8")
        for program in re.findall(
            r"^\s*```python[^\n]*\n(.*?)^\s*```", text, re.MULTILINE | re.DOTALL
        ):
            snippets += 1
            try:
                ast.parse(textwrap.dedent(program), filename=str(source))
            except SyntaxError as error:
                errors.append(f"{source.relative_to(ROOT)}: invalid Python snippet: {error.msg}")
    print(
        json.dumps(
            {
                "pages": len(corpus),
                "local_links": checked_links,
                "python_snippets": snippets,
                "errors": errors,
            },
            indent=2,
        )
    )
    if errors:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
