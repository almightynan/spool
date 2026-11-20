#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Turn Cargo.lock into flatpak-builder sources for an offline build.

Every crates.io package becomes an archive source unpacked to
cargo/vendor/<name>-<version>, with the sha256 from Cargo.lock, and a
.cargo-checksum.json so cargo accepts the vendored copy. A cargo config makes
cargo use that directory instead of crates.io.

Usage: build-aux/cargo-sources.py Cargo.lock > build-aux/cargo-sources.json
"""
import json
import sys
import tomllib

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def main(lock_path):
    with open(lock_path, "rb") as f:
        lock = tomllib.load(f)
    sources = []
    for pkg in lock.get("package", []):
        if pkg.get("source") != REGISTRY:
            # Workspace members have no source; git sources are not used here.
            if "source" in pkg:
                sys.exit(f"unsupported source for {pkg['name']}: {pkg['source']}")
            continue
        name, version, checksum = pkg["name"], pkg["version"], pkg["checksum"]
        dest = f"cargo/vendor/{name}-{version}"
        sources.append({
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": dest,
        })
        sources.append({
            "type": "inline",
            "contents": json.dumps({"package": checksum, "files": {}}),
            "dest": dest,
            "dest-filename": ".cargo-checksum.json",
        })
    sources.append({
        "type": "inline",
        "contents": (
            '[source.vendored-sources]\ndirectory = "cargo/vendor"\n\n'
            '[source.crates-io]\nreplace-with = "vendored-sources"\n'
        ),
        "dest": "cargo",
        "dest-filename": "config.toml",
    })
    json.dump(sources, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "Cargo.lock")
