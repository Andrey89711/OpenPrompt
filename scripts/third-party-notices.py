#!/usr/bin/env python3
"""Collect real license texts from Cargo's locked dependency resolution."""

import argparse
import json
from pathlib import Path
import stat
import subprocess
import sys


def license_texts(package):
    """Return package-relative filenames and texts; never follow license links."""
    folder = Path(package["manifest_path"]).resolve(strict=True).parent
    candidates = {
        path for path in folder.iterdir()
        if path.name.lower().startswith(("license", "copying"))
    }
    if package.get("license_file"):
        candidates.add(folder / package["license_file"])

    texts = []
    for path in sorted(candidates, key=lambda item: str(item).casefold()):
        # Do not collect executables or scripts merely named license.*.
        if path.suffix.lower() not in ("", ".txt", ".md"):
            continue
        try:
            resolved = path.resolve(strict=True)
            relative = resolved.relative_to(folder)
            if not stat.S_ISREG(path.lstat().st_mode):
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            if text.strip():
                texts.append((relative.as_posix(), text))
        except (OSError, ValueError) as error:
            print(
                f"warning: {package['name']} {package['version']}: "
                f"cannot collect {path}: {error}",
                file=sys.stderr,
            )
    return texts


def render_notices(metadata):
    resolution = metadata.get("resolve")
    if not resolution or not resolution.get("root"):
        raise ValueError("Cargo metadata must identify a resolved root package")
    root = resolution["root"]
    nodes = {node["id"]: node for node in resolution["nodes"]}
    reachable = set()
    pending = [root]
    while pending:
        package_id = pending.pop()
        if package_id not in reachable:
            reachable.add(package_id)
            pending.extend(nodes[package_id]["dependencies"])
    packages = sorted(
        (package for package in metadata["packages"]
         if package["id"] in reachable and package["id"] != root),
        key=lambda package: (package["name"], package["version"], package["id"]),
    )
    sections = [
        "THIRD-PARTY NOTICES\n\n"
        "License texts for dependencies reachable in Cargo's locked target resolution.\n"
        "May also include build/test dependencies for that target.\n"
        "Texts below are copied from the dependency packages; no license text is synthesized.\n"
    ]
    missing = []
    for package in packages:
        texts = license_texts(package)
        if not texts:
            missing.append(f"{package['name']} {package['version']} ({package['manifest_path']})")
            continue
        sections.append(
            f"\n{'=' * 78}\n{package['name']} {package['version']}\n"
            f"SPDX: {package.get('license') or 'not declared in Cargo metadata'}\n"
        )
        for filename, text in texts:
            sections.append(f"\n--- {filename} ---\n{text.rstrip()}\n")
    if missing:
        raise ValueError(
            "No readable license text for resolved dependencies:\n  " + "\n  ".join(missing)
        )
    return "".join(sections), len(packages)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "-o", "--output", type=Path, default=Path("THIRD_PARTY_NOTICES.txt"),
        help="output path (default: THIRD_PARTY_NOTICES.txt)",
    )
    parser.add_argument("--target", help="Cargo target triple (defaults to rustc host)")
    arguments = parser.parse_args()
    # target is supplied by the release matrix or defaults to the local host.
    try:
        target = arguments.target
        if target is None:
            version = subprocess.run(["rustc", "-vV"], check=True, stdout=subprocess.PIPE)
            target = next(line.removeprefix("host: ") for line in version.stdout.decode("utf-8").splitlines() if line.startswith("host: "))
        result = subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target],
            check=True, stdout=subprocess.PIPE,
        )
        metadata = json.loads(result.stdout.decode("utf-8"))
        notices, count = render_notices(metadata)
        # Do not create a partial notices file when any dependency is missing text.
        arguments.output.write_text(notices, encoding="utf-8")
    except (OSError, ValueError, KeyError, StopIteration, subprocess.CalledProcessError) as error:
        print(f"third-party-notices: {error}", file=sys.stderr)
        return 1
    print(f"Wrote notices for {count} dependencies to {arguments.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
