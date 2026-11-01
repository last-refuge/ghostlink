"""Collect every native artifact and write a deterministic public checksum manifest."""
import hashlib
from pathlib import Path
import shutil
import sys


def prepare(source: Path, output: Path, installer: Path) -> None:
    assets = {
        "ghostlink-windows-x86_64-setup.exe": "*-windows-x86_64-setup.exe",
        "ghostlink-windows-x86_64.zip": "*-windows-x86_64.zip",
        "ghostlink-macos-x86_64.zip": "*-macos-x86_64.zip",
        "ghostlink-macos-aarch64.zip": "*-macos-aarch64.zip",
        "ghostlink-linux-x86_64.tar.gz": "*-linux-x86_64.tar.gz",
        "ghostlink-linux-aarch64.tar.gz": "*-linux-aarch64.tar.gz",
        "ghostlink-arch-x86_64.pkg.tar.zst": "*-x86_64.pkg.tar.zst",
    }
    # Validate the full set before creating a release payload.
    selected = {}
    for name, pattern in assets.items():
        matches = list(source.rglob(pattern))
        if len(matches) != 1 or not matches[0].is_file() or matches[0].stat().st_size == 0:
            raise ValueError(f"Expected one nonempty {pattern}, found {matches}")
        selected[name] = matches[0]
    selected["install.sh"] = installer
    output.mkdir(parents=True, exist_ok=False)
    checksums = []
    for name, path in sorted(selected.items()):
        target = output / name
        shutil.copyfile(path, target)
        with target.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        checksums.append(f"{digest}  {name}\n")
    (output / "SHA256SUMS").write_text("".join(checksums), encoding="utf-8")


if __name__ == "__main__":
    prepare(*(Path(arg) for arg in sys.argv[1:]))
