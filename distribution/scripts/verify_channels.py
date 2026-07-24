#!/usr/bin/env python3
"""Prove direct, npm-global, and pinned-npx candidates against one Core fixture."""

import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

from common import CandidateError, executable_name, release_manifest


def run(command, *, env=None, cwd=None, capture=True):
    completed = subprocess.run(
        [str(value) for value in command],
        cwd=str(cwd) if cwd else None,
        env=env,
        check=False,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE,
        text=True,
    )
    if completed.returncode != 0:
        raise CandidateError(
            "command failed ({}): {}\n{}".format(
                completed.returncode,
                " ".join(str(value) for value in command),
                completed.stderr.strip(),
            )
        )
    return completed.stdout if capture else ""


def diagnostic(command, *, env=None):
    """Capture one expected CLI failure without normalizing its boundary."""

    completed = subprocess.run(
        [str(value) for value in command],
        env=env,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return {
        "returncode": completed.returncode,
        "stderr": completed.stderr,
        "stdout": completed.stdout,
    }


def channel_diagnostic_environments(
    direct: Path,
    npm_global: Path,
    npx_environment,
    homebrew: Path = None,
):
    """Authorize the exact binary exercised by each installed-provider fixture."""

    environments = {"npx": npx_environment}
    for name, binary in {"direct": direct, "npm_global": npm_global}.items():
        environment = os.environ.copy()
        environment["NOSTDB_BIN"] = str(binary)
        environments[name] = environment
    if homebrew is not None:
        environment = os.environ.copy()
        environment["NOSTDB_BIN"] = str(homebrew)
        environments["homebrew"] = environment
    return environments


def extract(archive: Path, destination: Path) -> Path:
    destination.mkdir(parents=True, exist_ok=False)

    def write_member(name, data):
        output = (destination / name).resolve()
        try:
            output.relative_to(destination.resolve())
        except ValueError as error:
            raise CandidateError("archive member escapes extraction root") from error
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(data)

    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as source:
            for member in source.infolist():
                if member.is_dir():
                    continue
                write_member(member.filename, source.read(member))
    else:
        with tarfile.open(archive, mode="r:gz") as source:
            for member in source.getmembers():
                if not member.isfile():
                    raise CandidateError("direct archive contains a non-file member")
                extracted = source.extractfile(member)
                if extracted is None:
                    raise CandidateError("cannot read direct archive member")
                write_member(member.name, extracted.read())
    candidates = list(destination.rglob("nostdb")) + list(destination.rglob("nostdb.exe"))
    if len(candidates) != 1:
        raise CandidateError("direct archive does not contain exactly one CLI")
    binary = candidates[0]
    if os.name != "nt":
        binary.chmod(binary.stat().st_mode | stat.S_IXUSR)
    return binary


def npm_bin(prefix: Path, global_install: bool) -> Path:
    if os.name == "nt":
        return prefix / ("nostdb.cmd" if global_install else "node_modules/.bin/nostdb.cmd")
    return prefix / ("bin/nostdb" if global_install else "node_modules/.bin/nostdb")


def write_npx_shim(directory: Path, real_npx: str, prefix: Path) -> None:
    version = release_manifest()["version"]
    python_script = directory / "npx_shim.py"
    python_script.write_text(
        "import os, subprocess, sys\n"
        f"expected = ['--yes', '--package=@nostdb/cli@{version}', 'nostdb']\n"
        "if sys.argv[1:4] != expected:\n"
        "    print('unexpected pinned npx command: ' + repr(sys.argv[1:]), file=sys.stderr)\n"
        "    sys.exit(97)\n"
        "command = [os.environ['REAL_NPX'], '--yes', '--offline', '--prefix', "
        "os.environ['NPX_PREFIX'], 'nostdb'] + sys.argv[4:]\n"
        "sys.exit(subprocess.run(command).returncode)\n",
        encoding="utf-8",
    )
    if os.name == "nt":
        real_cli = Path(real_npx).parent / "node_modules" / "npm" / "bin" / "npx-cli.js"
        if not real_cli.is_file():
            raise CandidateError("cannot locate the real Windows npx Node CLI")
        cli = directory / "node_modules" / "npm" / "bin" / "npx-cli.js"
        cli.parent.mkdir(parents=True)
        cli.write_text(
            "const { spawnSync } = require('node:child_process');\n"
            f"const expected = ['--yes', '--package=@nostdb/cli@{version}', 'nostdb'];\n"
            "if (JSON.stringify(process.argv.slice(2, 5)) !== JSON.stringify(expected)) {\n"
            "  console.error('unexpected pinned npx command'); process.exit(97);\n"
            "}\n"
            "const args = ['--yes', '--offline', '--prefix', process.env.NPX_PREFIX, "
            "'nostdb', ...process.argv.slice(5)];\n"
            "const result = spawnSync(process.execPath, [process.env.REAL_NPX_CLI, ...args], "
            "{stdio: 'inherit'});\n"
            "process.exit(result.status === null ? 3 : result.status);\n",
            encoding="utf-8",
        )
        (directory / "npx.cmd").write_text(
            "@rem npx is executed through node_modules/npm/bin/npx-cli.js\n",
            encoding="utf-8",
        )
    else:
        shim = directory / "npx"
        shim.write_text(
            "#!{}\nexec(open({!r}).read())\n".format(
                sys.executable, str(python_script)
            ),
            encoding="utf-8",
        )
        shim.chmod(0o755)


def fixture(
    skills: Path,
    output: Path,
    *,
    binary: Path = None,
    provider: str = "installed",
    env=None,
):
    command = [
        sys.executable,
        skills / "adapters" / "codex" / "run_fixture.py",
        "--fixture",
        skills / "tests" / "fixtures" / "portable",
        "--output",
        output,
        "--core-provider",
        provider,
    ]
    if binary:
        command.extend(["--binary", binary])
    return json.loads(run(command, env=env))


def fixture_diagnostic(project: Path, *, env=None):
    """Run an invalid query through the provider installed in one fixture."""

    return diagnostic(
        [
            sys.executable,
            project / ".agents" / "skills" / "nostdb" / "scripts" / "nostdb_core.py",
            "run",
            "--project",
            project,
            "--",
            "query",
            "MATCH (",
            "--database",
            project / ".nostdb",
            "--format",
            "json",
        ],
        env=env,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--native-evidence", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--skills-root", type=Path, required=True)
    parser.add_argument("--homebrew-binary", type=Path)
    args = parser.parse_args()
    try:
        skills = args.skills_root.resolve()
        if not (skills / "adapters/codex/run_fixture.py").is_file():
            raise CandidateError("invalid skills root: {}".format(skills))
        real_npx = shutil.which("npx")
        if real_npx is None:
            raise CandidateError("npx is required for channel verification")
        with tempfile.TemporaryDirectory(prefix="nostdb-channel-verification-") as temporary:
            root = Path(temporary)
            candidate = root / "candidate"
            archive = Path(
                run(
                    [
                        sys.executable,
                        Path(__file__).with_name("assemble_candidate.py"),
                        "--target",
                        args.target,
                        "--binary",
                        args.binary,
                        "--native-evidence",
                        args.native_evidence,
                        "--metadata",
                        args.metadata,
                        "--output",
                        candidate,
                    ]
                ).strip()
            )
            run(
                [
                    sys.executable,
                    Path(__file__).with_name("verify_candidate.py"),
                    "--archive",
                    archive,
                ]
            )
            direct = extract(archive, root / "direct")
            npm_output = root / "npm"
            npm_payload = json.loads(
                run(
                    [
                        sys.executable,
                        Path(__file__).with_name("stage_npm_candidate.py"),
                        "--target",
                        args.target,
                        "--binary",
                        args.binary,
                        "--output",
                        npm_output,
                    ]
                )
            )
            launcher = npm_output / npm_payload["launcher"]["filename"]
            platform = npm_output / npm_payload["platform"]["filename"]
            global_prefix = root / "npm-global"
            local_prefix = root / "npm-local"
            install_arguments = [
                "--ignore-scripts",
                "--omit=optional",
                "--offline",
                "--no-audit",
                "--no-fund",
                platform,
                launcher,
            ]
            run(
                ["npm", "install", "--global", "--prefix", global_prefix]
                + install_arguments
            )
            run(["npm", "install", "--prefix", local_prefix] + install_arguments)
            global_binary = npm_bin(global_prefix, True)
            local_binary = npm_bin(local_prefix, False)
            for channel, binary in {
                "direct": direct,
                "npm_global": global_binary,
                "npm_local": local_binary,
            }.items():
                version = run([binary, "--version"])
                if version != "nostdb {}\n".format(release_manifest()["version"]):
                    raise CandidateError("{} version mismatch".format(channel))
            results = {
                "direct": fixture(skills, root / "fixture-direct", binary=direct),
                "npm_global": fixture(
                    skills, root / "fixture-npm-global", binary=global_binary
                ),
            }
            shim_directory = root / "npx-shim"
            shim_directory.mkdir()
            write_npx_shim(shim_directory, real_npx, local_prefix)
            npx_environment = os.environ.copy()
            npx_environment.pop("NOSTDB_BIN", None)
            npx_environment["PATH"] = str(shim_directory) + os.pathsep + os.environ["PATH"]
            npx_environment["REAL_NPX"] = real_npx
            if os.name == "nt":
                npx_environment["REAL_NPX_CLI"] = str(
                    Path(real_npx).parent / "node_modules" / "npm" / "bin" / "npx-cli.js"
                )
            npx_environment["NPX_PREFIX"] = str(local_prefix)
            results["npx"] = fixture(
                skills,
                root / "fixture-npx",
                provider="npx",
                env=npx_environment,
            )
            if args.homebrew_binary:
                results["homebrew"] = fixture(
                    skills,
                    root / "fixture-homebrew",
                    binary=args.homebrew_binary.resolve(),
                )
            diagnostic_environments = channel_diagnostic_environments(
                direct,
                global_binary,
                npx_environment,
                args.homebrew_binary.resolve() if args.homebrew_binary else None,
            )
            diagnostics = {
                name: fixture_diagnostic(
                    root / "fixture-{}".format(name.replace("_", "-")),
                    env=diagnostic_environments.get(name),
                )
                for name in results
            }
            baseline = results["direct"]
            different = [name for name, result in results.items() if result != baseline]
            if different:
                raise CandidateError(
                    "channel fixture mismatch: {}".format(", ".join(different))
                )
            diagnostic_baseline = diagnostics["direct"]
            different_diagnostics = [
                name
                for name, result in diagnostics.items()
                if result != diagnostic_baseline
            ]
            if diagnostic_baseline["returncode"] == 0:
                raise CandidateError("invalid-query diagnostic unexpectedly succeeded")
            if different_diagnostics:
                raise CandidateError(
                    "channel diagnostic mismatch: {}; observed={}".format(
                        ", ".join(different_diagnostics),
                        json.dumps(
                            {
                                "direct": diagnostic_baseline,
                                **{
                                    name: diagnostics[name]
                                    for name in different_diagnostics
                                },
                            },
                            sort_keys=True,
                        ),
                    )
                )
            print(
                json.dumps(
                    {
                        "channels": sorted(results),
                        "diagnostic": diagnostic_baseline,
                        "fixture": baseline,
                        "published": False,
                        "target": args.target,
                    },
                    sort_keys=True,
                )
            )
        return 0
    except (CandidateError, OSError, ValueError, subprocess.SubprocessError) as error:
        print("nostdb-channel-verifier: {}".format(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
