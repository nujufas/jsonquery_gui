#!/usr/bin/env python3
"""Validate the winget manifests against Microsoft's published JSON schemas.

    validate-manifest.py [MANIFEST_DIR]        (default: the directory above this one)

Every manifest file names its own ManifestType and ManifestVersion, so the matching
schema (microsoft/winget-cli, schemas/JSON/manifests/v<ver>/manifest.<type>.<ver>.json)
is fetched from GitHub automatically. Besides the schema it checks what the winget
pipeline also insists on: one version + one installer + one defaultLocale manifest, all
with the same PackageIdentifier and PackageVersion.

This is the Linux stand-in for `winget validate`; it cannot know about winget's own
extra rules, so still run tools/test-winget-manifest.ps1 on a real Windows.

Needs:  pip install pyyaml jsonschema
"""
import json
import sys
import urllib.request
from pathlib import Path

try:
    import jsonschema
    import yaml
except ImportError:
    # Exit code 3 = "could not run" (1 = manifests invalid, 2 = nothing to validate),
    # which submit-to-winget-pkgs.py relies on.
    print("needs pyyaml and jsonschema:  pip install pyyaml jsonschema", file=sys.stderr)
    sys.exit(3)

SCHEMA_URL = (
    "https://raw.githubusercontent.com/microsoft/winget-cli/master/"
    "schemas/JSON/manifests/v{v}/manifest.{t}.{v}.json"
)
REQUIRED_TYPES = ("version", "installer", "defaultLocale")


def load(path: Path) -> dict:
    # BaseLoader keeps every scalar a string (ReleaseDate, versions...), which is what
    # the schema's patterns expect; the manifests contain no numbers or booleans.
    return yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)


def main() -> int:
    directory = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    files = sorted(directory.glob("*.yaml"))
    if not files:
        print(f"no *.yaml files in {directory}", file=sys.stderr)
        return 2

    schemas: dict[str, dict] = {}
    docs: dict[str, dict] = {}
    ok = True

    for f in files:
        doc = load(f)
        kind, ver = doc.get("ManifestType"), doc.get("ManifestVersion")
        if not kind or not ver:
            print(f"FAIL  {f.name}: no ManifestType / ManifestVersion")
            ok = False
            continue
        url = SCHEMA_URL.format(v=ver, t=kind)
        if url not in schemas:
            try:
                schemas[url] = json.load(urllib.request.urlopen(url, timeout=30))
            except OSError as exc:
                print(f"FAIL  {f.name}: cannot fetch schema {url} ({exc})")
                ok = False
                continue
        schema = schemas[url]
        validator = jsonschema.validators.validator_for(schema)(schema)
        errors = sorted(validator.iter_errors(doc), key=lambda e: list(e.path))
        if errors:
            ok = False
            print(f"FAIL  {f.name}  ({kind} schema {ver})")
            for e in errors:
                print(f"        {'/'.join(map(str, e.path)) or '<root>'}: {e.message}")
        else:
            print(f"ok    {f.name}  ({kind} schema {ver})")
        docs[f.name] = doc

    kinds = [d.get("ManifestType") for d in docs.values()]
    for t in REQUIRED_TYPES:
        if kinds.count(t) != 1:
            print(f"FAIL  expected exactly one '{t}' manifest, found {kinds.count(t)}")
            ok = False
    for key in ("PackageIdentifier", "PackageVersion"):
        values = {d.get(key) for d in docs.values()}
        if len(values) > 1:
            print(f"FAIL  {key} differs between files: {sorted(map(str, values))}")
            ok = False
    version_doc = next((d for d in docs.values() if d.get("ManifestType") == "version"), {})
    locale_doc = next((d for d in docs.values() if d.get("ManifestType") == "defaultLocale"), {})
    if version_doc.get("DefaultLocale") != locale_doc.get("PackageLocale"):
        print(
            f"FAIL  version manifest DefaultLocale {version_doc.get('DefaultLocale')!r} "
            f"!= defaultLocale manifest PackageLocale {locale_doc.get('PackageLocale')!r}"
        )
        ok = False

    print("VALID" if ok else "INVALID")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
