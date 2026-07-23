#!/usr/bin/env python3
"""Shared dependency-free release-candidate helpers."""

import hashlib
import json
import platform
import os
import subprocess
from pathlib import Path
from typing import Dict, Tuple


ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = ROOT / "distribution" / "release-manifest.json"


class CandidateError(RuntimeError):
    """An invalid or incomplete release candidate."""


def read_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise CandidateError("cannot read JSON {}: {}".format(path, error)) from error


def write_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def release_manifest() -> dict:
    manifest = read_json(MANIFEST_PATH)
    if manifest.get("schema_version") != 1:
        raise CandidateError("unsupported release manifest schema")
    return manifest


def target_details(target: str) -> Tuple[dict, dict]:
    manifest = release_manifest()
    try:
        details = manifest["targets"][target]
    except KeyError as error:
        raise CandidateError("unsupported release target: {}".format(target)) from error
    return manifest, details


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def host_target() -> str:
    systems: Dict[str, str] = {
        "Darwin": "apple-darwin",
        "Linux": "unknown-linux-gnu",
        "Windows": "pc-windows-msvc",
    }
    machines = {
        "aarch64": "aarch64",
        "arm64": "aarch64",
        "amd64": "x86_64",
        "x86_64": "x86_64",
    }
    system = platform.system()
    machine = platform.machine().lower()
    try:
        return "{}-{}".format(machines[machine], systems[system])
    except KeyError as error:
        raise CandidateError(
            "unsupported native host: {} {}".format(system, platform.machine())
        ) from error


def is_translated_process() -> bool:
    if platform.system() == "Darwin":
        completed = subprocess.run(
            ["sysctl", "-in", "sysctl.proc_translated"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        return completed.returncode == 0 and completed.stdout.strip() == "1"
    if platform.system() == "Windows":
        native = os.environ.get("PROCESSOR_ARCHITEW6432", "").lower()
        current = os.environ.get("PROCESSOR_ARCHITECTURE", "").lower()
        return bool(native and native != current)
    return False


def executable_name(target: str) -> str:
    return "nostdb.exe" if "windows" in target else "nostdb"


def archive_name(version: str, target: str, archive_kind: str) -> str:
    suffix = ".zip" if archive_kind == "zip" else ".tar.gz"
    return "nostdb-{}-{}{}".format(version, target, suffix)
