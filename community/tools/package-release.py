#!/usr/bin/env python3
"""Build one ZIP per package directory and the release marketplace.json index."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile
from urllib.parse import quote, urlparse
import zipfile

REPOSITORY = "hidarina101327/mochi-community"
KINDS = {"documents": "document", "bases": "base", "canvases": "canvas", "workflows": "workflow", "templates": "template", "agents": "agent", "knowledge-bases": "knowledge-base", "plugins": "plugin"}
RESERVED = {"con", "prn", "aux", "nul"} | {f"{p}{n}" for p in ("com", "lpt") for n in range(1, 10)}
MAX_BYTES = 128 * 1024 * 1024


def safe_id(value):
    return isinstance(value, str) and bool(re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,78}[a-z0-9])?", value)) and value not in RESERVED


def read_package(directory, kind):
    data = json.loads((directory / "marketplace.json").read_text(encoding="utf-8-sig"))
    if not safe_id(data.get("id")) or data["id"] != directory.name:
        raise ValueError(f"{directory}: id must match the package directory")
    if data.get("kind", kind) != kind:
        raise ValueError(f"{directory}: kind does not match the parent directory")
    for field in ("title", "version", "summary"):
        if not isinstance(data.get(field), str) or not data[field].strip():
            raise ValueError(f"{directory}: missing {field}")
    data["kind"] = kind
    data.setdefault("category", kind)
    if data["category"] not in KINDS.values():
        raise ValueError(f"{directory}: unknown category")
    data.setdefault("official", False)
    if not isinstance(data["official"], bool):
        raise ValueError(f"{directory}: official must be a boolean")
    for field in ("description", "author"):
        data.setdefault(field, "")
        if not isinstance(data[field], str):
            raise ValueError(f"{directory}: {field} must be text")
    if kind == "plugin":
        manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8-sig"))
        if manifest.get("id") != data["id"] or manifest.get("version") != data["version"]:
            raise ValueError(f"{directory}: plugin manifest identity differs from marketplace metadata")
    if kind == "workflow":
        json.loads((directory / "workflow.json").read_text(encoding="utf-8-sig"))
    if kind == "template" and not any(p.is_file() and p.suffix in (".md", ".mc", ".txt") for p in directory.iterdir()):
        raise ValueError(f"{directory}: template needs a root-level md/mc/txt file")
    return data


def package_files(directory):
    files, seen, size = [], set(), 0
    for path in sorted(directory.rglob("*")):
        relative = path.relative_to(directory)
        if path.is_symlink():
            raise ValueError(f"{path}: symbolic links are not allowed")
        for part in relative.parts:
            if part.endswith((".", " ")) or ":" in part or "\\" in part or part.split(".")[0].lower() in RESERVED:
                raise ValueError(f"{path}: unsafe Windows path")
        if any(p in (".git", "node_modules", "__pycache__") for p in relative.parts):
            continue
        name = relative.as_posix().lower()
        if name in seen:
            raise ValueError(f"{path}: duplicate case-insensitive path")
        seen.add(name)
        if path.is_file():
            size += path.stat().st_size
            files.append(path)
    if len(files) > 10000 or size > MAX_BYTES * 2:
        raise ValueError(f"{directory}: package exceeds extraction limits")
    return files


def build(root, output, tag):
    root, output = Path(root).resolve(), Path(output).resolve()
    if not tag or any(c in tag for c in "\r\n\\"):
        raise ValueError("A valid release tag is required")
    if output.exists() and any(output.iterdir()):
        raise ValueError("Output directory must be empty")
    packages = []
    with tempfile.TemporaryDirectory(prefix="mochi-community-build-") as temporary:
        staging = Path(temporary)
        for plural, kind in KINDS.items():
            parent = root / plural
            if parent.is_symlink():
                raise ValueError(f"{parent}: symbolic links are not allowed")
            if not parent.exists():
                continue
            for directory in sorted(parent.iterdir()):
                if directory.is_symlink():
                    raise ValueError(f"{directory}: symbolic links are not allowed")
                if not directory.is_dir():
                    continue
                data = read_package(directory, kind)
                files = package_files(directory)
                asset = f"{data['id']}-{kind}.zip"
                archive = staging / asset
                with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zip_file:
                    for path in files:
                        info = zipfile.ZipInfo(path.relative_to(directory).as_posix(), date_time=(1980, 1, 1, 0, 0, 0))
                        info.compress_type = zipfile.ZIP_DEFLATED
                        info.external_attr = 0o100644 << 16
                        zip_file.writestr(info, path.read_bytes())
                if archive.stat().st_size > MAX_BYTES:
                    raise ValueError(f"{archive}: ZIP exceeds download limit")
                image = data.get("image")
                if image:
                    if not isinstance(image, str):
                        raise ValueError("image must be text")
                    if image.startswith("https://"):
                        parsed = urlparse(image)
                        if not parsed.hostname or parsed.username:
                            raise ValueError("Invalid image URL")
                    else:
                        source = (directory / image).resolve()
                        if not source.is_relative_to(directory.resolve()) or source not in files or source.suffix.lower() not in (".png", ".jpg", ".jpeg", ".webp"):
                            raise ValueError(f"{directory}: image must be a packaged raster image or HTTPS URL")
                        image_asset = f"{data['id']}-{kind}-cover{source.suffix.lower()}"
                        shutil.copyfile(source, staging / image_asset)
                        image = f"https://github.com/{REPOSITORY}/releases/download/{quote(tag, safe='')}/{image_asset}"
                else:
                    image = None
                item = {k: data[k] for k in ("id", "kind", "category", "title", "version", "summary", "description", "official", "author")}
                item.update(image=image, asset=asset, sha256=hashlib.sha256(archive.read_bytes()).hexdigest())
                packages.append(item)
        if not packages:
            raise ValueError("No package directories found; refusing to publish an empty catalog")
        (staging / "marketplace.json").write_text(json.dumps({"schemaVersion": 1, "packages": packages}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        output.mkdir(parents=True, exist_ok=True)
        for path in staging.iterdir():
            shutil.copyfile(path, output / path.name)
    return packages


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    items = build(args.root, args.output, args.tag)
    print(f"Built {len(items)} independent packages in {args.output}")
