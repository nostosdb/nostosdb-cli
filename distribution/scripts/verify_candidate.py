#!/usr/bin/env python3
"""Verify one direct-download candidate without executing foreign bytes."""

import argparse
import json
import sys
import tarfile
import zipfile
from pathlib import Path

from common import CandidateError, read_json, sha256


REQUIRED = {
    "LICENSE",
    "NOTICE",
    "NATIVE_TEST_EVIDENCE.json",
    "README.md",
    "SBOM.spdx.json",
    "THIRD_PARTY_LICENSES.json",
}


def archive_contents(path: Path):
    if path.name.endswith(".zip"):
        with zipfile.ZipFile(path) as archive:
            return {name: archive.read(name) for name in archive.namelist()}
    with tarfile.open(path, mode="r:gz") as archive:
        return {
            member.name: archive.extractfile(member).read()
            for member in archive.getmembers()
            if member.isfile()
        }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    try:
        archive = args.archive.resolve()
        record = read_json(archive.with_name(archive.name + ".manifest.json"))
        if record.get("published") is not False:
            raise CandidateError("candidate manifest must record published=false")
        if record.get("archive_sha256") != sha256(archive):
            raise CandidateError("candidate archive checksum mismatch")
        checksum = archive.with_name(archive.name + ".sha256").read_text(
            encoding="utf-8"
        )
        expected_line = "{}  {}\n".format(record["archive_sha256"], archive.name)
        if checksum != expected_line:
            raise CandidateError("checksum sidecar mismatch")
        contents = archive_contents(archive)
        names = list(contents)
        roots = {name.split("/", 1)[0] for name in names}
        if len(roots) != 1:
            raise CandidateError("archive must contain exactly one root directory")
        relative = {name.split("/", 1)[1] for name in names if "/" in name}
        binary = "nostdb.exe" if "windows" in record["target"] else "nostdb"
        required = REQUIRED | {binary}
        if relative != required:
            raise CandidateError(
                "archive members mismatch: expected {}, found {}".format(
                    sorted(required), sorted(relative)
                )
            )
        root = next(iter(roots))
        sbom = json.loads(contents["{}/SBOM.spdx.json".format(root)])
        attribution = json.loads(
            contents["{}/THIRD_PARTY_LICENSES.json".format(root)]
        )
        evidence = json.loads(
            contents["{}/NATIVE_TEST_EVIDENCE.json".format(root)]
        )
        if sbom.get("spdxVersion") != "SPDX-2.3":
            raise CandidateError("candidate SBOM is not SPDX 2.3 JSON")
        if attribution.get("metadata_version") != 1:
            raise CandidateError("candidate attribution metadata is unsupported")
        if (
            evidence.get("target") != record["target"]
            or evidence.get("passed") is not True
            or evidence.get("translated") is not False
        ):
            raise CandidateError("candidate native evidence is invalid")
        print("verified {}".format(archive.name))
        return 0
    except (CandidateError, OSError, ValueError, KeyError, tarfile.TarError) as error:
        print("nostdb-candidate-verifier: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
