#!/usr/bin/env python3
"""Build and prove all non-Homebrew channels on the current native host."""

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

from common import CandidateError, ROOT, executable_name, host_target


def run(command, cwd=ROOT):
    completed = subprocess.run(
        [str(value) for value in command],
        cwd=str(cwd),
        check=False,
    )
    if completed.returncode != 0:
        raise CandidateError(
            "command failed ({}): {}".format(
                completed.returncode, " ".join(str(value) for value in command)
            )
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skills-root", type=Path, required=True)
    args = parser.parse_args()
    try:
        target = host_target()
        run(["cargo", "build", "--release", "--all-features", "--locked"])
        binary = ROOT / "target" / "release" / executable_name(target)
        with tempfile.TemporaryDirectory(prefix="nostdb-local-distribution-") as temporary:
            output = Path(temporary)
            metadata = output / "metadata"
            evidence = output / "native.json"
            run(
                [
                    sys.executable,
                    Path(__file__).with_name("generate_metadata.py"),
                    "--output",
                    metadata,
                ]
            )
            run(
                [
                    sys.executable,
                    Path(__file__).with_name("smoke_candidate.py"),
                    "--target",
                    target,
                    "--binary",
                    binary,
                    "--output",
                    evidence,
                ]
            )
            run(
                [
                    sys.executable,
                    Path(__file__).with_name("verify_channels.py"),
                    "--target",
                    target,
                    "--binary",
                    binary,
                    "--native-evidence",
                    evidence,
                    "--metadata",
                    metadata,
                    "--skills-root",
                    args.skills_root.resolve(),
                ]
            )
        return 0
    except (CandidateError, OSError, subprocess.SubprocessError) as error:
        print("nostdb-local-distribution: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
