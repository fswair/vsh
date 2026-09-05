"""Generate LLM bundles and the exact page-source corpus for VSH documentation."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urljoin

ROOT = Path(__file__).resolve().parents[1]
DOCS_DIR = ROOT / "docs"
CONFIG_PATH = ROOT / "zensical.toml"
COMPACT_PATH = DOCS_DIR / "llms.txt"
FULL_PATH = DOCS_DIR / "llms-full.txt"
SOURCES_PATH = DOCS_DIR / "assets" / "markdown.json"


@dataclass(frozen=True)
class Page:
    """One source-backed documentation page."""

    section: str
    title: str
    source: Path
    url: str
    summary: str


def strip_front_matter(markdown: str) -> str:
    """Remove a leading YAML metadata block without changing the page body."""
    lines = markdown.splitlines()
    if not lines or lines[0].strip() != "---":
        return markdown.strip()
    for index, line in enumerate(lines[1:], start=1):
        if line.strip() == "---":
            return "\n".join(lines[index + 1 :]).strip()
    raise ValueError("unterminated Markdown front matter")


def plain_text(value: str) -> str:
    """Collapse the small Markdown/HTML subset used in titles and summaries."""
    value = re.sub(r"\[([^]]+)]\([^)]*\)", r"\1", value)
    value = re.sub(r"<[^>]+>", " ", value)
    value = value.replace("`", "").replace("*", "").replace("_", "")
    return re.sub(r"\s+", " ", value).strip()


def page_title(body: str, fallback: str) -> str:
    """Read the first Markdown or HTML H1, falling back to the nav label."""
    heading = re.search(r"^#\s+(.+?)\s*$", body, flags=re.MULTILINE)
    if heading:
        return plain_text(heading.group(1))
    html_heading = re.search(r"<h1[^>]*>(.*?)</h1>", body, flags=re.DOTALL | re.IGNORECASE)
    if html_heading:
        return plain_text(html_heading.group(1))
    return fallback


def page_summary(body: str, fallback: str) -> str:
    """Extract the first prose paragraph for the compact index."""
    paragraph: list[str] = []
    in_fence = False
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if line.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if not line:
            if paragraph:
                break
            continue
        if line.startswith(("#", "- ", "* ", ">", "|", "===", "!!!", "???")):
            if paragraph:
                break
            continue
        if line.startswith("<"):
            if paragraph:
                break
            continue
        paragraph.append(line)

    summary = plain_text(" ".join(paragraph)) or fallback
    if len(summary) <= 200:
        return summary
    shortened = summary[:197].rsplit(" ", maxsplit=1)[0]
    return f"{shortened}..."


def page_url(site_url: str, reference: str) -> str:
    """Map a docs source path to the directory URL emitted by Zensical."""
    source = Path(reference)
    if source.name == "index.md":
        route = "" if source.parent == Path(".") else f"{source.parent.as_posix()}/"
    else:
        route = f"{source.with_suffix('').as_posix()}/"
    return urljoin(site_url, route)


def collect_pages(nav: list[Any], site_url: str, description: str) -> list[Page]:
    """List nav pages first, then include every remaining Markdown source."""
    pages: list[Page] = []
    seen: set[Path] = set()

    def add(reference: str, label: str | None, section: str) -> None:
        source = DOCS_DIR / reference
        if source in seen:
            return
        if not source.is_file():
            raise FileNotFoundError(f"navigation source does not exist: {source}")
        body = strip_front_matter(source.read_text(encoding="utf-8"))
        title = page_title(body, label or source.stem.replace("-", " ").title())
        pages.append(
            Page(
                section=section,
                title=title,
                source=source,
                url=page_url(site_url, reference),
                summary=page_summary(body, description),
            )
        )
        seen.add(source)

    def visit(items: list[Any], section: str) -> None:
        for item in items:
            if isinstance(item, str):
                add(item, None, section)
                continue
            if not isinstance(item, dict):
                raise TypeError(f"unsupported navigation item: {item!r}")
            for label, target in item.items():
                if isinstance(target, str):
                    add(target, str(label), section)
                elif isinstance(target, list):
                    visit(target, str(label))
                else:
                    raise TypeError(f"unsupported navigation target: {target!r}")

    visit(nav, "Overview")
    for source in sorted(DOCS_DIR.rglob("*.md")):
        if source in seen:
            continue
        add(source.relative_to(DOCS_DIR).as_posix(), None, "Additional reference")
    return pages


def render_compact(site_name: str, description: str, pages: list[Page]) -> str:
    """Render the llms.txt discovery index."""
    lines = [
        f"# {site_name}",
        "",
        f"> {description}",
        "",
        (
            "Use the links below for focused documentation. Use llms-full.txt when a single "
            "complete Markdown context is preferable."
        ),
    ]
    current_section = ""
    for page in pages:
        if page.section != current_section:
            current_section = page.section
            lines.extend(("", f"## {current_section}"))
        lines.extend(("", f"- [{page.title}]({page.url}): {page.summary}"))
    lines.extend(("", "## Full documentation", "", "- [llms-full.txt](llms-full.txt)"))
    return "\n".join(lines) + "\n"


def without_leading_title(body: str) -> str:
    """Avoid repeating a page H1 immediately below its full-bundle delimiter."""
    body = re.sub(r"^\s*#\s+.+?\n", "", body, count=1)
    return re.sub(
        r"^\s*<h1[^>]*>.*?</h1>\s*",
        "",
        body,
        count=1,
        flags=re.DOTALL | re.IGNORECASE,
    ).strip()


def render_full(site_name: str, description: str, pages: list[Page]) -> str:
    """Render every navigable source page as one deterministic Markdown corpus."""
    lines = [
        f"# {site_name} — Full documentation",
        "",
        f"> {description}",
        "",
        (
            "This file contains every documentation source: Zensical navigation pages first, "
            "then the remaining references."
        ),
        "",
        "## Contents",
    ]
    lines.extend(f"- [{page.title}]({page.url})" for page in pages)
    for page in pages:
        body = strip_front_matter(page.source.read_text(encoding="utf-8"))
        body = "\n".join(line.rstrip() for line in body.splitlines())
        lines.extend(
            (
                "",
                "---",
                "",
                f"# {page.title}",
                "",
                f"Canonical URL: {page.url}",
                "",
                without_leading_title(body),
            )
        )
    return "\n".join(lines).rstrip() + "\n"


def sync_file(path: Path, expected: str, *, check: bool) -> bool:
    """Write a generated file or report that its committed copy is stale."""
    current = path.read_text(encoding="utf-8") if path.exists() else None
    if current == expected:
        return True
    if check:
        print(f"{path.relative_to(ROOT)} is stale; regenerate LLM documentation", file=sys.stderr)
        return False
    path.write_text(expected, encoding="utf-8")
    print(f"generated {path.relative_to(ROOT)}")
    return True


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail when the committed outputs differ from the generated content",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    with CONFIG_PATH.open("rb") as config_file:
        project = tomllib.load(config_file)["project"]

    site_name = str(project["site_name"])
    site_url = str(project["site_url"])
    description = str(project["site_description"])
    nav = project["nav"]
    if not isinstance(nav, list):
        raise TypeError("project.nav must be a list")

    pages = collect_pages(nav, site_url, description)
    results = (
        sync_file(COMPACT_PATH, render_compact(site_name, description, pages), check=args.check),
        sync_file(FULL_PATH, render_full(site_name, description, pages), check=args.check),
        sync_file(
            SOURCES_PATH,
            json.dumps(
                {
                    page.url.removeprefix(site_url): page.source.read_text(encoding="utf-8")
                    for page in pages
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            check=args.check,
        ),
    )
    return 0 if all(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
