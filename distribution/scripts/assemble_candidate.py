#!/usr/bin/env python3
"""Assemble one deterministic, target-native-proven Nostos CLI candidate."""

import argparse
import gzip
import io
import json
import stat
import sys
import tarfile
import zipfile
from pathlib import Path

from common import (
    CandidateError,
    ROOT,
    archive_name,
    executable_name,
    read_json,
    sha256,
    target_details,
    write_json,
)


def candidate_files(
    binary: Path, metadata: Path, native_evidence: Path, target: str
):
    return {
        executable_name(target): binary,
        "LICENSE": ROOT / "LICENSE",
        "NOTICE": ROOT / "NOTICE",
        "NATIVE_TEST_EVIDENCE.json": native_evidence,
        "README.md": ROOT / "README.md",
        "SBOM.spdx.json": metadata / "SBOM.spdx.json",
        "THIRD_PARTY_LICENSES.json": metadata / "THIRD_PARTY_LICENSES.json",
    }


def tar_gzip(path: Path, prefix: str, files) -> None:
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w", format=tarfile.PAX_FORMAT) as output:
        for name, source in sorted(files.items()):
            data = source.read_bytes()
            info = tarfile.TarInfo("{}/{}".format(prefix, name))
            info.size = len(data)
            info.mode = 0o755 if name == "nostos" else 0o644
            info.mtime = 0
            info.uid = 0
            info.gid = 0
            info.uname = ""
            info.gname = ""
            output.addfile(info, io.BytesIO(data))
    with path.open("wb") as destination:
        with gzip.GzipFile(fileobj=destination, mode="wb", filename="", mtime=0) as output:
            output.write(archive.getvalue())


def zip_archive(path: Path, prefix: str, files) -> None:
    with zipfile.ZipFile(path, mode="w", compression=zipfile.ZIP_DEFLATED) as output:
        for name, source in sorted(files.items()):
            info = zipfile.ZipInfo("{}/{}".format(prefix, name), (1980, 1, 1, 0, 0, 0))
            mode = 0o755 if name.endswith(".exe") else 0o644
            info.external_attr = (stat.S_IFREG | mode) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            output.writestr(info, source.read_bytes())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--native-evidence", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        manifest, details = target_details(args.target)
        evidence = read_json(args.native_evidence)
        required_evidence = {
            "host_target": args.target,
            "passed": True,
            "target": args.target,
            "translated": False,
            "version": manifest["version"],
        }
        for key, value in required_evidence.items():
            if evidence.get(key) != value:
                raise CandidateError(
                    "native evidence {} mismatch: expected {!r}, found {!r}".format(
                        key, value, evidence.get(key)
                    )
                )
        tests = evidence.get("tests", {})
        version_test = tests.get("version", {})
        help_test = tests.get("help", {})
        if (
            version_test.get("returncode") != 0
            or version_test.get("stdout") != "nostos {}\n".format(manifest["version"])
            or help_test.get("returncode") != 0
            or "Usage:" not in help_test.get("stdout", "")
        ):
            raise CandidateError("native evidence does not contain passing CLI tests")
        binary = args.binary.resolve()
        files = candidate_files(
            binary,
            args.metadata.resolve(),
            args.native_evidence.resolve(),
            args.target,
        )
        missing = [str(path) for path in files.values() if not path.is_file()]
        if missing:
            raise CandidateError("missing candidate input: {}".format(", ".join(missing)))
        args.output.mkdir(parents=True, exist_ok=True)
        name = archive_name(manifest["version"], args.target, details["archive"])
        archive = args.output / name
        prefix = name.removesuffix(".tar.gz").removesuffix(".zip")
        if details["archive"] == "zip":
            zip_archive(archive, prefix, files)
        else:
            tar_gzip(archive, prefix, files)
        checksum = sha256(archive)
        checksum_path = archive.with_name(archive.name + ".sha256")
        checksum_path.write_text(
            "{}  {}\n".format(checksum, archive.name), encoding="utf-8"
        )
        record = {
            "archive": archive.name,
            "archive_sha256": checksum,
            "binary_sha256": sha256(binary),
            "candidate_version": 1,
            "native_evidence_sha256": sha256(args.native_evidence),
            "npm_package": details["npm_package"],
            "published": False,
            "target": args.target,
            "version": manifest["version"],
        }
        write_json(archive.with_name(archive.name + ".manifest.json"), record)
        print(archive.resolve())
        return 0
    except (CandidateError, OSError, ValueError) as error:
        print("nostos-assemble: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
