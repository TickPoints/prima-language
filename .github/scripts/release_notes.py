#!/usr/bin/env python3
"""Extract the changelog section for a release tag as GitHub release notes.

Usage: release_notes.py <tag>

Uses the section matching the tag's version (a leading 'v' is stripped) in
docs/CHANGELOG.md. Falls back to the [Unreleased] section when the version
section has not been added yet. Exits non-zero when neither section exists.
"""

import re
import sys


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: release_notes.py <tag>")

    tag = sys.argv[1]
    version = tag.lstrip("v")

    with open("docs/CHANGELOG.md", encoding="utf-8") as fh:
        text = fh.read()

    parts = re.split(r"\n## \[([^\]]*)\][^\n]*\n", text)

    def section(name: str) -> str:
        try:
            idx = parts.index(name)
        except ValueError:
            return ""
        return parts[idx + 1].strip()

    body = section(version) or section("Unreleased")
    if not body:
        sys.exit(
            f"error: no changelog section for version '{version}' and no "
            "[Unreleased] section found in docs/CHANGELOG.md"
        )
    print(body)


if __name__ == "__main__":
    main()
