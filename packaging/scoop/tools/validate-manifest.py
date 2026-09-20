#!/usr/bin/env python3
"""Check the Scoop manifest against Scoop's JSON schema and against the real release.

    validate-manifest.py [MANIFEST.json]      (default: ../jsonquery-gui.json)
    validate-manifest.py --offline            skip the download + hash check

Three checks, all runnable on Linux:
  1. the manifest against Scoop's own schema (ScoopInstaller/Scoop, schema.json)
  2. the version is spelled the same in `version`, `url` and `extract_dir`
  3. the zip behind `url` downloads, hashes to `hash`, and holds `extract_dir/` with every
     file `bin` and `shortcuts` point at

This is the Linux stand-in for `scoop install`; tools/test-scoop-manifest.ps1 runs the real
thing on Windows.

Needs:  pip install jsonschema
"""
import hashlib
import io
import json
import sys
import urllib.request
import zipfile
from pathlib import Path

try:
    import jsonschema
except ImportError:
    print("needs jsonschema:  pip install jsonschema", file=sys.stderr)
    sys.exit(3)

SCHEMA_URL = "https://raw.githubusercontent.com/ScoopInstaller/Scoop/master/schema.json"


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "jsonquery-scoop-validate"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.read()


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    offline = "--offline" in sys.argv
    path = Path(args[0]) if args else Path(__file__).resolve().parent.parent / "jsonquery-gui.json"
    manifest = json.loads(path.read_text(encoding="utf-8"))
    ok = True

    schema = json.loads(fetch(SCHEMA_URL))
    errors = sorted(
        jsonschema.validators.validator_for(schema)(schema).iter_errors(manifest),
        key=lambda e: list(e.path),
    )
    if errors:
        ok = False
        print(f"FAIL  {path.name} against Scoop's schema")
        for e in errors:
            print(f"        {'/'.join(map(str, e.path)) or '<root>'}: {e.message}")
    else:
        print(f"ok    {path.name} matches Scoop's schema")

    version = manifest["version"]
    arch = manifest["architecture"]["64bit"]
    url = arch["url"]
    extract_dir = manifest.get("extract_dir", "")
    for label, value in (("url", url), ("extract_dir", extract_dir)):
        if version not in value:
            ok = False
            print(f"FAIL  version {version} does not appear in {label}: {value}")
    auto = manifest.get("autoupdate", {})
    auto_url = auto.get("architecture", {}).get("64bit", {}).get("url", "")
    if url != auto_url.replace("$version", version):
        ok = False
        print(f"FAIL  autoupdate url does not reproduce url:\n        {auto_url}\n        {url}")
    if extract_dir != auto.get("extract_dir", "").replace("$version", version):
        ok = False
        print("FAIL  autoupdate extract_dir does not reproduce extract_dir")
    if ok:
        print(f"ok    version {version} is consistent (version / url / extract_dir / autoupdate)")

    if offline:
        print("skipped the download check (--offline)")
    else:
        try:
            data = fetch(url)
        except OSError as exc:
            print(f"FAIL  cannot download {url} ({exc})")
            return 1
        digest = hashlib.sha256(data).hexdigest()
        if digest != arch["hash"].lower():
            ok = False
            print(f"FAIL  hash mismatch: manifest {arch['hash']}, download {digest}")
        else:
            print(f"ok    download hashes to the manifest's sha256 ({len(data) / 1e6:.1f} MB)")
        names = set(zipfile.ZipFile(io.BytesIO(data)).namelist())
        wanted = [b[0] if isinstance(b, list) else b for b in manifest.get("bin", [])]
        wanted += [s[0] for s in manifest.get("shortcuts", [])]
        for exe in dict.fromkeys(wanted):
            member = f"{extract_dir}/{exe}"
            if member in names:
                print(f"ok    zip contains {member}")
            else:
                ok = False
                print(f"FAIL  zip has no {member} (has: {sorted(names)})")

    print("VALID" if ok else "INVALID")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
