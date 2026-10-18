#!/usr/bin/env python3
"""Fetch published DOOP facts without adding generated data to the repository.

Usage: python3 features/datalog/doop/fetch.py cassandra [--cache /tmp/doop-data]
The command prints the extracted app directory; completed downloads are reused.
Source: https://github.com/flowlog-rs/flowlog-bench/blob/main/docs/data/doop-data-prep.md
Facts are tab-delimited <app>/<RelationName>.facts, including MainClass.facts.
"""

import argparse
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys
import tempfile
import urllib.error
import urllib.request
import zipfile


APPS = (
    "avrora", "batik", "biojava", "cassandra", "eclipse", "fop", "graphchi",
    "h2", "h2o", "jme", "jython", "kafka", "luindex", "lusearch", "pmd",
    "spring", "sunflow", "tomcat", "xalan", "zxing",
)
BASE_URL = (
    "https://huggingface.co/datasets/NemoYuu/flowlog_benchmark/"
    "resolve/main/dataset/csv"
)


def fetch(app: str, cache: Path) -> Path:
    cache = cache.expanduser().resolve()
    cache.mkdir(parents=True, exist_ok=True)
    destination = cache / app
    if destination.exists():
        if destination.is_dir() and (destination / "MainClass.facts").is_file():
            return destination
        raise ValueError(f"incomplete cache directory; remove it before fetching: {destination}")

    # Stage both download and extraction on the destination filesystem. Only a
    # fully extracted archive becomes visible as the cached app directory.
    with tempfile.TemporaryDirectory(prefix=f".{app}-", dir=cache) as temporary:
        stage = Path(temporary)
        archive = stage / f"{app}.zip"
        url = f"{BASE_URL}/{app}.zip"
        print(f"Downloading {url}", file=sys.stderr)
        with urllib.request.urlopen(url, timeout=120) as response, archive.open("wb") as output:
            shutil.copyfileobj(response, output, length=1024 * 1024)
        with zipfile.ZipFile(archive) as source:
            members = source.infolist()
            for member in members:
                path = PurePosixPath(member.filename)
                mode = member.external_attr >> 16
                if (
                    path.is_absolute()
                    or ".." in path.parts
                    or "\\" in member.filename
                    or not path.parts
                    or path.parts[0] != app
                    or stat.S_ISLNK(mode)
                    or (stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR))
                ):
                    raise ValueError(f"unsafe archive member: {member.filename!r}")
            source.extractall(stage, members=members)
        extracted = stage / app
        if not (extracted / "MainClass.facts").is_file():
            raise ValueError(f"archive does not contain {app}/MainClass.facts")
        extracted.rename(destination)
        # TemporaryDirectory removes the zip after successful extraction, and
        # also removes incomplete downloads/extractions on failure.
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", choices=APPS)
    parser.add_argument("--cache", type=Path, default=Path("/tmp/doop-data"))
    args = parser.parse_args()
    try:
        print(fetch(args.app, args.cache))
    except (OSError, ValueError, urllib.error.URLError, zipfile.BadZipFile) as error:
        print(f"fetch: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
