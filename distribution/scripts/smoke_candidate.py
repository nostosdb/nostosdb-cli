#!/usr/bin/env python3
"""Run target-native CLI smoke checks and emit required candidate evidence."""

import argparse
import subprocess
import sys
from pathlib import Path

from common import (
    CandidateError,
    host_target,
    is_translated_process,
    target_details,
    write_json,
)


def checked(command):
    completed = subprocess.run(
        [str(value) for value in command],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=60,
    )
    return {
        "arguments": [str(value) for value in command[1:]],
        "returncode": completed.returncode,
        "stderr": completed.stderr,
        "stdout": completed.stdout,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        manifest, _ = target_details(args.target)
        actual_target = host_target()
        if is_translated_process():
            raise CandidateError("native smoke refuses a translated/emulated process")
        if actual_target != args.target:
            raise CandidateError(
                "native smoke target mismatch: expected {}, host is {}".format(
                    args.target, actual_target
                )
            )
        binary = args.binary.resolve()
        if not binary.is_file():
            raise CandidateError("candidate binary does not exist: {}".format(binary))
        version = checked([binary, "--version"])
        help_result = checked([binary, "--help"])
        expected = "nostdb {}\n".format(manifest["version"])
        passed = (
            version["returncode"] == 0
            and version["stdout"] == expected
            and not version["stderr"]
            and help_result["returncode"] == 0
            and "Usage:" in help_result["stdout"]
        )
        evidence = {
            "evidence_version": 1,
            "host_target": actual_target,
            "passed": passed,
            "target": args.target,
            "translated": False,
            "tests": {"help": help_result, "version": version},
            "version": manifest["version"],
        }
        write_json(args.output, evidence)
        if not passed:
            raise CandidateError("native candidate smoke checks failed")
        print(args.output.resolve())
        return 0
    except (CandidateError, OSError, subprocess.TimeoutExpired) as error:
        print("nostdb-smoke: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
