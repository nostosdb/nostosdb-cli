#!/usr/bin/env python3
"""Render the project-owned Homebrew formula from two verified candidates."""

import argparse
import sys
from pathlib import Path

from common import CandidateError, ROOT, host_target, read_json, release_manifest, sha256


TARGETS = {
    "arm64": "aarch64-apple-darwin",
    "x64": "x86_64-apple-darwin",
}


def checked_archive(path: Path, target: str) -> tuple:
    path = path.resolve()
    record = read_json(path.with_name(path.name + ".manifest.json"))
    if record.get("target") != target or record.get("published") is not False:
        raise CandidateError("invalid Homebrew candidate for {}".format(target))
    digest = sha256(path)
    if record.get("archive_sha256") != digest:
        raise CandidateError("Homebrew candidate checksum mismatch")
    return path.as_uri(), digest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arm64-archive", type=Path)
    parser.add_argument("--x64-archive", type=Path)
    parser.add_argument(
        "--host-smoke-archive",
        type=Path,
        help="render a non-publishable formula for one native macOS candidate",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.host_smoke_archive:
            if args.arm64_archive or args.x64_archive:
                raise CandidateError(
                    "host smoke archive cannot be combined with release candidates"
                )
            target = host_target()
            if target not in TARGETS.values():
                raise CandidateError("Homebrew smoke verification requires native macOS")
            host_url, host_hash = checked_archive(args.host_smoke_archive, target)
            arm_url = x64_url = host_url
            arm_hash = x64_hash = host_hash
        else:
            if args.arm64_archive is None or args.x64_archive is None:
                raise CandidateError(
                    "both macOS candidates are required for a release formula"
                )
            arm_url, arm_hash = checked_archive(
                args.arm64_archive, TARGETS["arm64"]
            )
            x64_url, x64_hash = checked_archive(args.x64_archive, TARGETS["x64"])
        template = (
            ROOT / "distribution" / "homebrew" / "Formula" / "nostos.rb.in"
        ).read_text(encoding="utf-8")
        replacements = {
            "@ARM64_SHA256@": arm_hash,
            "@ARM64_URL@": arm_url,
            "@VERSION@": release_manifest()["version"],
            "@X64_SHA256@": x64_hash,
            "@X64_URL@": x64_url,
        }
        for marker, value in replacements.items():
            template = template.replace(marker, value)
        if args.host_smoke_archive:
            template = template.replace(
                "# Generated candidate template. Publication requires separate authorization.",
                "# HOST-ONLY SMOKE FORMULA. NEVER PUBLISH.\n"
                "# Both branches intentionally reference the one verified native host archive.",
            )
        if "@" in "".join(
            line for line in template.splitlines() if not line.lstrip().startswith("#")
        ):
            raise CandidateError("unresolved Homebrew formula marker")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(template, encoding="utf-8")
        print(args.output.resolve())
        return 0
    except (CandidateError, OSError, ValueError) as error:
        print("nostos-homebrew: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
