#!/usr/bin/env python3
"""Verify candidate source structure and its no-publication boundary."""

import json
import re
import sys
from pathlib import Path

from common import (
    CandidateError,
    ROOT,
    release_manifest,
)


PACKAGE_VERSION = re.compile(r'^version = "([^"]+)"$', re.MULTILINE)


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
            directory = package_name.replace("@nostdb/cli-", "")
            package = json.loads(
                (npm_root / "packages" / directory / "package.json").read_text(
                    encoding="utf-8"
                )
            )
            if package["name"] != package_name or package["version"] != manifest["version"]:
                raise CandidateError("invalid platform package {}".format(package_name))
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
        for path in sorted((ROOT / "distribution" / "scripts").glob("*.py")):
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
        print("nostdb-distribution-source: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
