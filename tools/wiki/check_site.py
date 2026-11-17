#!/usr/bin/env python3
"""Check every link in the built site.

    python3 tools/wiki/check_site.py target/wiki/site/book --site-url=/meridian-core/

tools/check_links.py checks the Markdown in the repository. This checks what is
actually published: each page mdBook wrote is parsed, and every link, image,
stylesheet and script that points inside the site must exist, and every
`#fragment` must be an id on the page it points at. Links to other sites are
listed with --external so another tool can fetch them; they are not fetched
here, so this check cannot fail because somebody else's server is down.

--site-url is the path the site is served under (`site-url` in book.toml). A
link that starts with it, as the links on the 404 page do, is looked up from
the top of the built site.
"""
from __future__ import annotations

import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

# One page holding every chapter, for printing. Its links are the chapters'
# links re-based by mdBook, and each chapter is already checked on its own.
SKIP = {"print.html"}


class Page(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.ids: set[str] = set()
        self.links: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if values.get("id"):
            self.ids.add(values["id"])
        if tag == "a" and values.get("name"):
            self.ids.add(values["name"])
        for attr in ("href", "src"):
            if values.get(attr):
                self.links.append(values[attr])


def parse(path: Path) -> Page:
    page = Page()
    page.feed(path.read_text(encoding="utf-8"))
    return page


def main(argv: list[str]) -> int:
    args = [a for a in argv[1:] if not a.startswith("--")]
    site_url = next((a.split("=", 1)[1] for a in argv[1:] if a.startswith("--site-url=")), "/")
    site_url = "/" + site_url.strip("/") + "/" if site_url.strip("/") else "/"
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    site = Path(args[0]).resolve()
    if not (site / "index.html").exists():
        print(f"{site} is not a built site: it has no index.html", file=sys.stderr)
        return 2

    pages = {p: parse(p) for p in sorted(site.rglob("*.html")) if p.name not in SKIP}
    failures: list[str] = []
    external: set[str] = set()
    checked = 0

    for path, page in pages.items():
        here = path.relative_to(site)
        for link in page.links:
            parts = urlsplit(link)
            if parts.scheme in ("http", "https"):
                external.add(link)
                continue
            if parts.scheme or link.startswith("//"):
                continue  # mailto:, javascript:, data:
            checked += 1
            if parts.path.startswith("/"):
                if not parts.path.startswith(site_url):
                    failures.append(f"{here}: leaves the site -> {link}")
                    continue
                target = (site / unquote(parts.path[len(site_url):])).resolve()
            else:
                target = path if not parts.path else (path.parent / unquote(parts.path)).resolve()
            if target.is_dir():
                target = target / "index.html"
            if not target.exists():
                failures.append(f"{here}: missing file -> {link}")
                continue
            fragment = unquote(parts.fragment)
            if not fragment or target.suffix != ".html":
                continue
            if target not in pages:
                pages[target] = parse(target)
            if fragment not in pages[target].ids:
                failures.append(f"{here}: missing anchor -> {link}")

    if "--external" in argv:
        for link in sorted(external):
            print(link)
        return 0
    for failure in failures:
        print(failure, file=sys.stderr)
    print(f"checked {checked} links in {len(pages)} pages, {len(failures)} broken; "
          f"{len(external)} distinct external links not fetched")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
