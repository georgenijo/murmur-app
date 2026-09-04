#!/usr/bin/env python3
"""Verify local Markdown links in maintained docs resolve to real targets.

Scans AGENTS.md, README.md, and the maintained `docs/` tree for Markdown
links, and fails on any local (non-external) link whose target file,
directory, or in-file heading anchor does not exist. External URLs
(http(s), mailto, etc.) are ignored, as are archived/raw material
directories that are not part of the maintained documentation set.
"""

from __future__ import annotations

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]

# Root-level Markdown files that are part of the maintained documentation set.
ROOT_DOCS = (
    ROOT / "AGENTS.md",
    ROOT / "README.md",
    ROOT / "CLAUDE.md",
    ROOT / "CHANGELOG.md",
    ROOT / "THIRD_PARTY_NOTICES.md",
)

# Directories under docs/ that hold archived, raw, or point-in-time material
# rather than maintained documentation: broken links inside them are not
# actionable the way a stale doc-tree link is, so they are ignored here.
ARCHIVE_DIRS = {
    "archive",
    "draft",
    "evidence",
    "investigations",
    "qa",
    "reports",
    "research",
    "sessions",
}

# Individual docs/ files that are point-in-time research/execution artifacts
# rather than maintained documentation.
ARCHIVE_FILES = {
    "research-structured-events.md",
    "PAPERCLIP_FAMILY_HOST_PROOF.md",
}

LINK_RE = re.compile(r"(?<!!)\[[^\]\n]*\]\(([^)]+)\)")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*$", re.MULTILINE)
EXTERNAL_SCHEME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")


def maintained_docs(root: Path = ROOT) -> list[Path]:
    docs = [
        root / relative
        for path in ROOT_DOCS
        if (root / (relative := path.relative_to(ROOT))).exists()
    ]
    docs_dir = root / "docs"
    for path in sorted(docs_dir.rglob("*.md")):
        relative = path.relative_to(docs_dir)
        if relative.name in ARCHIVE_FILES:
            continue
        if relative.parts[0] in ARCHIVE_DIRS if len(relative.parts) > 1 else False:
            continue
        docs.append(path)
    return docs


def slugify(heading: str) -> str:
    # Inline code spans are literal text, not markdown emphasis: `hang_diagnostics.rs`
    # must keep its underscore even though bare `_word_` elsewhere means italics.
    parts = re.split(r"(`[^`]*`)", heading)
    text = "".join(
        part[1:-1] if part.startswith("`") and part.endswith("`") and len(part) >= 2
        else re.sub(r"[*_~]", "", part)
        for part in parts
    )
    text = text.strip().lower()
    text = re.sub(r"[^\w\s-]", "", text)
    text = re.sub(r"\s+", "-", text)
    return text


def heading_anchors(text: str) -> set[str]:
    counts: dict[str, int] = {}
    anchors = set()
    for _, heading in HEADING_RE.findall(text):
        slug = slugify(heading)
        seen = counts.get(slug, 0)
        counts[slug] = seen + 1
        anchors.add(slug if seen == 0 else f"{slug}-{seen}")
    return anchors


def is_external(target: str) -> bool:
    if not target:
        return True
    if target.startswith("//"):
        return True
    return bool(EXTERNAL_SCHEME_RE.match(target))


def strip_title(target: str) -> str:
    # A markdown link destination may carry a trailing `"title"`.
    match = re.match(r'^(\S+)(?:\s+"[^"]*")?$', target.strip())
    return match.group(1) if match else target.strip()


def find_broken_links(paths: list[Path], root: Path = ROOT) -> list[tuple[Path, str, str]]:
    broken = []
    for path in paths:
        text = path.read_text()
        for target in LINK_RE.findall(text):
            target = strip_title(target)
            if is_external(target):
                continue
            path_part, _, anchor = target.partition("#")
            if not path_part:
                target_file = path
            else:
                target_file = (path.parent / path_part).resolve()
                if root not in target_file.parents and target_file != root:
                    broken.append((path, target, "escapes repository root"))
                    continue
                if not target_file.exists():
                    broken.append((path, target, "no such file or directory"))
                    continue
            if anchor and target_file.is_file() and target_file.suffix == ".md":
                anchors = heading_anchors(text if target_file == path else target_file.read_text())
                if anchor not in anchors:
                    broken.append((path, target, f"no heading matches #{anchor}"))
    return broken


def validate_markdown_links(root: Path = ROOT) -> int:
    paths = maintained_docs(root)
    broken = find_broken_links(paths, root)
    if broken:
        details = "\n".join(
            f"  {path.relative_to(root)}: [{target}] — {reason}"
            for path, target, reason in broken
        )
        raise AssertionError(f"broken local Markdown links found:\n{details}")
    return len(paths)


def main() -> None:
    count = validate_markdown_links()
    print(f"markdown links valid: {count} maintained docs scanned")


if __name__ == "__main__":
    main()
