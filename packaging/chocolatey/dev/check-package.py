#!/usr/bin/env python3
"""Pre-flight checks for the Chocolatey package, everything that can run on Linux.

    check-package.py [--offline]

What it verifies:
  * the nuspec parses and carries the fields the community repository's validator and
    moderators ask for (description >= 30 characters, tags, project/licence/source URLs, ...)
  * the version is the same in the nuspec, tools/chocolateyInstall.ps1, the release-notes and
    icon URLs
  * the release zip named in the install script downloads, hashes to `checksum64`, and holds
    the exe the script goes looking for
  * the URLs in the nuspec answer (the packageSourceUrl only exists once the folder is pushed
    to GitHub, so that one is a warning)
  * only the two chocolatey*.ps1 scripts sit in tools/ (everything there is packed)

`choco pack` (dev/choco.sh pack) and the VM test (dev/test-choco-package.ps1) cover the rest.
Stdlib only. Exit code 0 = fine (warnings allowed), 1 = a check failed.
"""
import hashlib
import io
import re
import sys
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
NS = "{http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd}"
REQUIRED = (
    "id", "version", "title", "authors", "owners", "projectUrl", "iconUrl", "licenseUrl",
    "projectSourceUrl", "packageSourceUrl", "docsUrl", "bugTrackerUrl", "copyright", "tags",
    "summary", "description", "releaseNotes",
)
URL_FIELDS = ("projectUrl", "iconUrl", "licenseUrl", "projectSourceUrl", "docsUrl", "bugTrackerUrl", "releaseNotes")

failed = False


def ok(msg: str) -> None:
    print(f"ok    {msg}")


def fail(msg: str) -> None:
    global failed
    failed = True
    print(f"FAIL  {msg}")


def warn(msg: str) -> None:
    print(f"warn  {msg}")


def get(url: str) -> tuple[int, str, bytes]:
    req = urllib.request.Request(url, headers={"User-Agent": "jsonquery-choco-check"})
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return resp.status, resp.headers.get("Content-Type", ""), resp.read()
    except urllib.error.HTTPError as exc:
        return exc.code, "", b""


def main() -> int:
    offline = "--offline" in sys.argv
    nuspec_path = next(PKG.glob("*.nuspec"))
    meta = ET.parse(nuspec_path).getroot().find(f"{NS}metadata")
    field = {name: (meta.findtext(f"{NS}{name}") or "").strip() for name in REQUIRED}

    missing = [n for n in REQUIRED if not field[n]]
    if missing:
        fail(f"empty or missing nuspec fields: {', '.join(missing)}")
    else:
        ok("all recommended nuspec fields are present")
    if len(field["description"]) < 30:
        fail("description is shorter than 30 characters (validator CPMR0032)")
    if not re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", field["id"]) or nuspec_path.stem != field["id"]:
        fail(f"id {field['id']!r} must be lowercase with hyphens and match the file name {nuspec_path.name}")
    else:
        ok(f"id {field['id']} matches the file name")
    version = field["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+(\.\d+)?", version):
        fail(f"version {version!r} is not a plain x.y.z")

    script = (PKG / "tools" / "chocolateyInstall.ps1").read_text(encoding="utf-8")
    script.encode("ascii")  # stays ASCII: PowerShell 5.1 reads BOM-less files as ANSI
    v = re.search(r"^\$version\s*=\s*'([^']+)'", script, re.M)
    c = re.search(r"^\$checksum64\s*=\s*'([0-9A-Fa-f]{64})'", script, re.M)
    f = re.search(r'^\$folder\s*=\s*"([^"]+)"', script, re.M)
    u = re.search(r'url64bit\s*=\s*"([^"]+)"', script)
    if not (v and c and f and u):
        fail("could not read $version / $checksum64 / $folder / url64bit from chocolateyInstall.ps1")
        print("INVALID")
        return 1
    if v.group(1) != version:
        fail(f"chocolateyInstall.ps1 says {v.group(1)} but the nuspec says {version}")
    else:
        ok(f"version {version} is the same in the nuspec and chocolateyInstall.ps1")
    for name in ("releaseNotes", "iconUrl"):
        if f"v{version}" not in field[name]:
            fail(f"{name} does not point at the v{version} tag: {field[name]}")
    if "checksumType64 = 'sha256'" not in script:
        fail("chocolateyInstall.ps1 must set checksumType64 = 'sha256'")

    tools = sorted(p.name for p in (PKG / "tools").iterdir())
    if tools != ["chocolateyInstall.ps1", "chocolateyUninstall.ps1"]:
        fail(f"tools/ holds {tools}; everything in it ends up in the package")
    else:
        ok("tools/ holds only the two chocolatey scripts")

    folder = f.group(1).replace("$version", v.group(1))
    url = u.group(1).replace("$folder", folder).replace("$version", v.group(1))
    if offline:
        print("skipped the network checks (--offline)")
    else:
        status, _, data = get(url)
        if status != 200:
            fail(f"cannot download {url} (HTTP {status})")
        else:
            digest = hashlib.sha256(data).hexdigest()
            if digest.lower() != c.group(1).lower():
                fail(f"checksum64 mismatch: script {c.group(1)}, download {digest}")
            else:
                ok(f"the release zip hashes to checksum64 ({len(data) / 1e6:.1f} MB)")
            member = f"{folder}/jsonquery_gui.exe"
            if member in zipfile.ZipFile(io.BytesIO(data)).namelist():
                ok(f"the zip contains {member}")
            else:
                fail(f"the zip has no {member}")
        for name in URL_FIELDS:
            status, ctype, _ = get(field[name])
            if status != 200:
                fail(f"{name} answers HTTP {status}: {field[name]}")
            elif name == "iconUrl" and not ctype.startswith("image/png"):
                fail(f"iconUrl is {ctype!r}, not a PNG")
        ok("nuspec URLs answer")
        status, _, _ = get(field["packageSourceUrl"])
        if status == 200:
            ok("packageSourceUrl answers")
        else:
            warn(f"packageSourceUrl answers HTTP {status} - push packaging/chocolatey to GitHub before submitting")

    print("INVALID" if failed else "VALID")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
