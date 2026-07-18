#!/usr/bin/env python3
"""Generate deterministic SPDX and third-party candidate metadata from Cargo."""

import argparse
import json
import subprocess
import sys
from pathlib import Path

from common import CandidateError, release_manifest, write_json


def license_expression(package: dict) -> str:
    value = package.get("license") or "NOASSERTION"
    return {"MIT/Apache-2.0": "MIT OR Apache-2.0"}.get(value, value)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        completed = subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=str(Path(__file__).resolve().parents[2]),
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if completed.returncode != 0:
            raise CandidateError(
                "cargo metadata failed: {}".format(completed.stderr.strip())
            )
        metadata = json.loads(completed.stdout)
        packages = sorted(
            metadata["packages"], key=lambda item: (item["name"], item["version"])
        )
        version = release_manifest()["version"]
        document = {
            "SPDXID": "SPDXRef-DOCUMENT",
            "creationInfo": {
                "creators": ["Tool: nostos-distribution-metadata-1"],
                "created": "1970-01-01T00:00:00Z",
            },
            "dataLicense": "CC0-1.0",
            "documentNamespace": (
                "https://github.com/nostosdb/nostosdb-cli/sbom/{}".format(version)
            ),
            "name": "nostos-cli-{}".format(version),
            "packages": [
                {
                    "SPDXID": "SPDXRef-Package-{}".format(index),
                    "downloadLocation": package.get("source") or "NOASSERTION",
                    "filesAnalyzed": False,
                    "licenseConcluded": license_expression(package),
                    "licenseDeclared": license_expression(package),
                    "name": package["name"],
                    "supplier": "NOASSERTION",
                    "versionInfo": package["version"],
                }
                for index, package in enumerate(packages, 1)
            ],
            "spdxVersion": "SPDX-2.3",
        }
        third_party = {
            "metadata_version": 1,
            "notice": (
                "Candidate inventory only; counsel-approved license text bundling "
                "remains a release gate."
            ),
            "packages": [
                {
                    "authors": package.get("authors") or [],
                    "homepage": package.get("homepage"),
                    "license": license_expression(package),
                    "name": package["name"],
                    "repository": package.get("repository"),
                    "source": package.get("source") or "workspace",
                    "version": package["version"],
                }
                for package in packages
                if package.get("source")
            ],
        }
        write_json(args.output / "SBOM.spdx.json", document)
        write_json(args.output / "THIRD_PARTY_LICENSES.json", third_party)
        print(
            json.dumps(
                {"packages": len(packages), "third_party": len(third_party["packages"])},
                sort_keys=True,
            )
        )
        return 0
    except (CandidateError, OSError, ValueError) as error:
        print("nostos-metadata: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
