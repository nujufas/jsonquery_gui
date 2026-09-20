#!/usr/bin/env python3
"""Submit the manifests in packaging/winget/ to microsoft/winget-pkgs as a pull request.

    submit-to-winget-pkgs.py [MANIFEST_DIR] [options]

      --dry-run             read-only: show what would be pushed and the PR text, change nothing
      --no-pr               push the branch to your fork but do not open the pull request
      --body-file FILE      use FILE as the PR description instead of the generated one
      --note TEXT           add a sentence to the generated description
      --cla-signed          tick "Signed the CLA" in the generated checklist
      --validated-locally   tick "Validated manifest locally with winget validate"
      --tested-locally      tick "Tested manifest locally with winget install --manifest"
      --skip-validate       do not run validate-manifest.py first

The boxes are only ticked for things this script can verify itself (no other open PRs,
one manifest, schema check passed) or that you assert with a flag. Re-running after
you changed the manifests pushes a new commit to the same branch, which updates the PR.

Nothing is cloned: winget-pkgs is enormous, so your fork is edited through the GitHub API
(blobs -> tree -> one commit -> branch). Needs the GitHub CLI `gh`, logged in with the
`repo` scope. Manifests are stored with CRLF line endings in winget-pkgs, so that is what
gets committed (the copies in this repo stay LF).
"""
import argparse
import base64
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

UPSTREAM = "microsoft/winget-pkgs"
GH = os.environ.get("GH", "gh")  # overridable so the script can be tested against a fake
TOOLS_DIR = Path(__file__).resolve().parent


class GhError(RuntimeError):
    pass


def gh(*args: str, body=None) -> str:
    proc = subprocess.run(
        [GH, *args],
        input=None if body is None else json.dumps(body),
        capture_output=True,
        text=True,
    )
    if proc.returncode:
        raise GhError((proc.stderr or proc.stdout).strip())
    return proc.stdout


def api(path: str, method: str = "GET", body=None, missing_ok: bool = False):
    args = ["api", path] if method == "GET" and body is None else ["api", "-X", method, path]
    if body is not None:
        args += ["--input", "-"]
    try:
        out = gh(*args, body=body)
    except GhError as exc:
        if missing_ok and "HTTP 404" in str(exc):
            return None
        raise
    return json.loads(out) if out.strip() else {}


def field(text: str, key: str) -> str | None:
    m = re.search(rf"^{key}:\s*(\S+)", text, re.M)
    return m.group(1) if m else None


def dest_dir(package_id: str, version: str) -> str:
    """manifests/<first letter>/<Publisher>/<Package>/<version>; every dot is a directory."""
    parts = package_id.split(".")
    return "/".join(["manifests", parts[0][0].lower(), *parts, version])


def to_crlf(data: bytes) -> bytes:
    return re.sub(rb"\r?\n", b"\r\n", data)


def pr_title(package_id: str, version: str, is_new: bool) -> str:
    # Formats from winget-pkgs' PR template: "New package: Publisher.Name version X.Y.Z"
    # or "Update: Publisher.Name to X.Y.Z".
    return f"New package: {package_id} version {version}" if is_new else f"Update: {package_id} to {version}"


def default_body(title: str, schema: str, note: str, ticks: dict) -> str:
    x = lambda key: "x" if ticks.get(key) else " "  # noqa: E731
    minor = ".".join(schema.split(".")[:2])
    lines = [
        "## 📖 Description",
        f"{title}.",
        "",
    ]
    if note:
        lines += [note, ""]
    lines += [
        "## ✅ Checklist",
        "",
        f"- [{x('cla')}] Signed the [Contributor License Agreement](https://cla.opensource.microsoft.com)",
        "- [ ] Linked to an issue (if applicable)",
        "  - N/A",
        "",
        "## 📦 Manifest Checklist",
        "",
        f"- [{x('no_other_prs')}] Checked that there aren't other open "
        "[pull requests](https://github.com/microsoft/winget-pkgs/pulls) for the same manifest update/change",
        "- [x] This PR only modifies one (1) manifest",
        f"- [{x('validated')}] Validated manifest locally with `winget validate --manifest <path>` "
        "([validation guide](https://github.com/microsoft/winget-pkgs/blob/master/doc/ValidationFailureGuide.md))",
        f"- [{x('tested')}] Tested manifest locally with `winget install --manifest <path>`",
        f"- [{x('schema')}] Manifest conforms to the "
        f"[{minor} schema](https://github.com/microsoft/winget-pkgs/tree/master/doc/manifest/schema/{schema})",
        "",
        "> **Note:** `<path>` is the directory containing the manifest you're submitting.",
        "",
    ]
    return "\n".join(lines)


def preflight_validate(manifest_dir: Path) -> bool | None:
    """True = passed, None = could not run (missing deps), exits on failure."""
    proc = subprocess.run([sys.executable, str(TOOLS_DIR / "validate-manifest.py"), str(manifest_dir)])
    if proc.returncode == 0:
        return True
    if proc.returncode == 3:
        print("warning: schema validation skipped (see above); the schema box stays unticked", file=sys.stderr)
        return None
    sys.exit("the manifests do not validate - fix them first (or --skip-validate)")


def wait_for_fork(fork: str, branch: str, timeout: int = 180) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if api(f"repos/{fork}/git/ref/heads/{branch}", missing_ok=True) is not None:
            return
        time.sleep(4)
    sys.exit(f"the fork {fork} did not become ready within {timeout}s")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("manifest_dir", nargs="?", type=Path, default=TOOLS_DIR.parent)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--no-pr", action="store_true")
    ap.add_argument("--body-file", type=Path)
    ap.add_argument("--note", default="")
    ap.add_argument("--cla-signed", action="store_true")
    ap.add_argument("--validated-locally", action="store_true")
    ap.add_argument("--tested-locally", action="store_true")
    ap.add_argument("--skip-validate", action="store_true")
    args = ap.parse_args()

    files = sorted(args.manifest_dir.glob("*.yaml"))
    if not files:
        sys.exit(f"no *.yaml manifests in {args.manifest_dir}")
    installer = next((f for f in files if f.name.endswith(".installer.yaml")), None)
    if installer is None:
        sys.exit("no *.installer.yaml among the manifests")
    text = installer.read_text(encoding="utf-8")
    package_id, version, schema = (field(text, k) for k in ("PackageIdentifier", "PackageVersion", "ManifestVersion"))
    if not (package_id and version and schema):
        sys.exit("PackageIdentifier / PackageVersion / ManifestVersion missing from the installer manifest")
    dest = dest_dir(package_id, version)

    schema_ok = None if args.skip_validate else preflight_validate(args.manifest_dir)

    login = api("user")["login"]
    fork = f"{login}/winget-pkgs"
    upstream = api(f"repos/{UPSTREAM}")
    base_branch = upstream["default_branch"]
    branch = f"{package_id}-{version}"

    is_new = api(f"repos/{UPSTREAM}/contents/{dest.rsplit('/', 1)[0]}", missing_ok=True) is None
    if api(f"repos/{UPSTREAM}/contents/{dest}", missing_ok=True) is not None:
        sys.exit(f"{package_id} {version} already exists in {UPSTREAM} - bump PackageVersion first")
    title = pr_title(package_id, version, is_new)

    prs = json.loads(
        gh(
            "pr", "list", "--repo", UPSTREAM, "--state", "open", "--limit", "30",
            "--search", f'"{package_id}" in:title',
            "--json", "number,title,url,headRefName,headRepositoryOwner",
        )
        or "[]"
    )
    mine = [p for p in prs if p["headRefName"] == branch and p["headRepositoryOwner"]["login"] == login]
    # Only a PR for the *same* version is a duplicate; one for another version may stay open.
    others = [p for p in prs if p not in mine and version in p["title"]]
    if others:
        print(f"other open PRs already submit {package_id} {version}:", file=sys.stderr)
        for p in others:
            print(f"  #{p['number']} {p['title']}  {p['url']}", file=sys.stderr)
        sys.exit("resolve those first")

    fork_info = api(f"repos/{fork}", missing_ok=True)
    if fork_info is not None and fork_info.get("parent", {}).get("full_name") != UPSTREAM:
        sys.exit(f"{fork} exists but is not a fork of {UPSTREAM}")
    branch_ref = api(f"repos/{fork}/git/ref/heads/{branch}", missing_ok=True) if fork_info else None

    ticks = {
        "cla": args.cla_signed,
        "no_other_prs": True,  # checked just above
        "validated": args.validated_locally,
        "tested": args.tested_locally,
        "schema": schema_ok is True,
    }
    body = args.body_file.read_text(encoding="utf-8") if args.body_file else default_body(title, schema, args.note, ticks)

    print(f"package : {package_id} {version}  ({'new package' if is_new else 'update of an existing package'})")
    print(f"target  : {UPSTREAM}:{dest}/")
    print(f"fork    : {fork}  ({'exists' if fork_info else 'will be created'}), branch {branch} "
          f"({'exists - a new commit is added on top' if branch_ref else 'new'})")
    for f in files:
        print(f"file    : {f.name}  ({len(to_crlf(f.read_bytes()))} bytes, CRLF)")
    print(f"PR      : {title}" + (f"   (updates open PR #{mine[0]['number']})" if mine else ""))
    if args.dry_run:
        print("\n--- PR description ---\n" + body)
        print("(dry run: nothing was changed)")
        return 0

    if fork_info is None:
        print("creating the fork ...")
        gh("repo", "fork", UPSTREAM, "--clone=false")
        wait_for_fork(fork, base_branch)
    else:
        try:  # bring the fork's default branch up to date so the PR has a fresh base
            api(f"repos/{fork}/merge-upstream", "POST", {"branch": base_branch})
        except GhError as exc:
            print(f"warning: could not sync the fork's {base_branch}: {exc}", file=sys.stderr)

    if branch_ref:
        base_commit = branch_ref["object"]["sha"]
    else:
        base_commit = api(f"repos/{fork}/git/ref/heads/{base_branch}")["object"]["sha"]
    base_tree = api(f"repos/{fork}/git/commits/{base_commit}")["tree"]["sha"]

    entries = []
    for f in files:
        blob = api(
            f"repos/{fork}/git/blobs", "POST",
            {"content": base64.b64encode(to_crlf(f.read_bytes())).decode(), "encoding": "base64"},
        )
        entries.append({"path": f"{dest}/{f.name}", "mode": "100644", "type": "blob", "sha": blob["sha"]})
    tree = api(f"repos/{fork}/git/trees", "POST", {"base_tree": base_tree, "tree": entries})

    if branch_ref and tree["sha"] == base_tree:
        print(f"branch {branch} already holds exactly these files - nothing to push")
    else:
        commit = api(
            f"repos/{fork}/git/commits", "POST",
            {"message": title + "\n", "tree": tree["sha"], "parents": [base_commit]},
        )
        if branch_ref:
            api(f"repos/{fork}/git/refs/heads/{branch}", "PATCH", {"sha": commit["sha"]})
        else:
            api(f"repos/{fork}/git/refs", "POST", {"ref": f"refs/heads/{branch}", "sha": commit["sha"]})
        print(f"pushed {commit['sha'][:10]} to {fork}:{branch}")

    if mine:
        print(f"open PR #{mine[0]['number']} picks the new commit up: {mine[0]['url']}")
    elif args.no_pr:
        print(f"branch is ready: https://github.com/{fork}/tree/{branch}")
    else:
        with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False, encoding="utf-8") as tmp:
            tmp.write(body)
        try:
            url = gh(
                "pr", "create", "--repo", UPSTREAM, "--base", base_branch,
                "--head", f"{login}:{branch}", "--title", title, "--body-file", tmp.name,
            ).strip()
        finally:
            os.unlink(tmp.name)
        print(f"opened {url}")
        if not args.cla_signed:
            print("The CLA bot will ask you to reply '@microsoft-github-policy-service agree' on the PR.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except GhError as exc:
        sys.exit(f"gh failed: {exc}")
    except FileNotFoundError:
        sys.exit(f"'{GH}' (GitHub CLI) not found - install it and run `gh auth login`")
