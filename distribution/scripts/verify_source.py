#!/usr/bin/env python3
"""Verify candidate source structure and its no-publication boundary."""

import json
import re
import sys
from pathlib import Path

from common import (
    CandidateError,
    ROOT,
    archive_name,
    executable_name,
    release_manifest,
)


PACKAGE_VERSION = re.compile(r'^version = "([^"]+)"$', re.MULTILINE)
MATRIX_ENTRY = re.compile(
    r"^\s+- runner: (?P<runner>\S+)\n"
    r"\s+target: (?P<target>\S+)\n"
    r"\s+binary: (?P<binary>\S+)\n"
    r"\s+archive: (?P<archive>\S+)$",
    re.MULTILINE,
)


def candidate_matrix(workflow: str) -> dict:
    """Extract the fixed native build matrix without a YAML dependency."""

    entries = {}
    for match in MATRIX_ENTRY.finditer(workflow):
        target = match.group("target")
        if target in entries:
            raise CandidateError("candidate workflow repeats target {}".format(target))
        entries[target] = {
            "archive": match.group("archive"),
            "binary": match.group("binary"),
            "runner": match.group("runner"),
        }
    return entries


def main() -> int:
    try:
        manifest = release_manifest()
        cargo_text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        match = PACKAGE_VERSION.search(cargo_text)
        if not match or match.group(1) != manifest["version"]:
            raise CandidateError("Cargo and distribution versions differ")
        npm_root = ROOT / "npm"
        launcher = json.loads((npm_root / "package.json").read_text(encoding="utf-8"))
        if launcher["version"] != manifest["version"]:
            raise CandidateError("npm launcher and distribution versions differ")
        expected_packages = {
            details["npm_package"] for details in manifest["targets"].values()
        }
        if set(launcher["optionalDependencies"]) != expected_packages:
            raise CandidateError("npm optional packages do not match release targets")
        for package_name in expected_packages:
            directory = package_name.replace("@nostosdb/cli-", "")
            package = json.loads(
                (npm_root / "packages" / directory / "package.json").read_text(
                    encoding="utf-8"
                )
            )
            if package["name"] != package_name or package["version"] != manifest["version"]:
                raise CandidateError("invalid platform package {}".format(package_name))
        workflow = (
            ROOT / ".github" / "workflows" / "attest-candidate.yml"
        ).read_text(encoding="utf-8")
        expected_matrix = {
            target: {
                "archive": archive_name(
                    manifest["version"], target, details["archive"]
                ),
                "binary": executable_name(target),
                "runner": details["runner"],
            }
            for target, details in manifest["targets"].items()
        }
        actual_matrix = candidate_matrix(workflow)
        if actual_matrix != expected_matrix:
            raise CandidateError(
                "candidate workflow matrix differs from release manifest"
            )
        if "actions/setup-python@" not in workflow or "python3 " in workflow:
            raise CandidateError(
                "candidate workflow must configure and use cross-platform Python"
            )
        forbidden = (
            "npm publish",
            "cargo publish",
            "git push",
            "gh release create",
            "gh release upload",
            "brew tap-new",
            "softprops/action-gh-release@",
            "ncipollo/release-action@",
            "docker push",
        )
        for path in [
            ROOT / ".github" / "workflows" / "attest-candidate.yml",
            *sorted((ROOT / "distribution" / "scripts").glob("*.py")),
        ]:
            if path.resolve() == Path(__file__).resolve():
                continue
            text = path.read_text(encoding="utf-8")
            for marker in forbidden:
                if marker in text:
                    raise CandidateError(
                        "{} contains forbidden external action {}".format(path, marker)
                    )
        print(
            "verified {} targets, {} npm packages, and no publication commands".format(
                len(manifest["targets"]), len(expected_packages) + 1
            )
        )
        return 0
    except (CandidateError, OSError, ValueError, KeyError) as error:
        print("nostos-distribution-source: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
