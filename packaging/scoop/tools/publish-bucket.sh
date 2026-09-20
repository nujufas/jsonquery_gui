#!/usr/bin/env bash
# Publish the Scoop manifest to its bucket repo, creating the repo from Scoop's
# BucketTemplate the first time (the template brings the CI tests and the Excavator
# workflow that bumps the manifest by itself when a new release appears).
#
#   publish-bucket.sh [--dry-run] [--force] [--repo OWNER/NAME]
#
#   --dry-run   change nothing on GitHub: build the bucket contents in a temp dir (from the
#               existing bucket, or the public template), style-check them, show the result
#   --force     push even if the bucket already holds a newer version than packaging/scoop
#   --repo      the bucket repository (default: nujufas/scoop-jsonquery-gui)
#
# Environment: BUCKET_COMMIT_TRAILER  an extra trailer line for the commit message.
# Needs: gh (logged in, `repo` scope), git, python3 + jsonschema (for the pre-flight check).
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCOOP_DIR="$(cd "$TOOLS_DIR/.." && pwd)" # packaging/scoop
TEMPLATE="ScoopInstaller/BucketTemplate"
APP="jsonquery-gui"

REPO="nujufas/scoop-jsonquery-gui"
DRY=0
FORCE=0
while [ $# -gt 0 ]; do
    case "$1" in
    --dry-run) DRY=1 ;;
    --force) FORCE=1 ;;
    --repo) shift && REPO="${1:?--repo needs OWNER/NAME}" ;;
    -h | --help)
        awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"
        exit 0
        ;;
    *)
        echo "unknown option: $1" >&2
        exit 2
        ;;
    esac
    shift
done

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
manifest_version() { sed -n 's/^[[:space:]]*"version":[[:space:]]*"\([^"]*\)".*/\1/p' "$1" | head -1; }

# Put our manifest and README into a checkout of the bucket (or of the template).
customise() {
    local dir="$1" branch="$2"
    rm -f "$dir/bucket/app-name.json.template" "$dir/.github/ISSUE_TEMPLATE/package-request.yml"
    mkdir -p "$dir/bucket"
    cp "$SCOOP_DIR/$APP.json" "$dir/bucket/$APP.json"
    sed "s#{{REPO}}#$REPO#g" "$SCOOP_DIR/bucket-README.md" >"$dir/README.md"
    # bin/auto-pr.ps1 carries a placeholder upstream ("<username>/<bucketname>:main")
    sed -i "s#<username>/<bucketname>:[A-Za-z0-9_.-]*#$REPO:$branch#" "$dir/bin/auto-pr.ps1"
    grep -q "$REPO:$branch" "$dir/bin/auto-pr.ps1" || die "could not set the upstream in bin/auto-pr.ps1"
}

# The bucket's CI checks every file (Scoop-00File.Tests.ps1); catch what it would catch.
style_check() {
    local dir="$1" f bad=0
    for f in "bucket/$APP.json" README.md bin/auto-pr.ps1; do
        [ -z "$(tail -c1 "$dir/$f")" ] || { echo "  $f: no final newline"; bad=1; }
        ! grep -q -P '[ \t]+$' "$dir/$f" || { echo "  $f: trailing whitespace"; bad=1; }
        ! grep -q -P '^ *\t' "$dir/$f" || { echo "  $f: tab indentation"; bad=1; }
        ! head -c3 "$dir/$f" | grep -q -P '^\xEF\xBB\xBF' || { echo "  $f: UTF-8 BOM"; bad=1; }
    done
    return "$bad"
}

command -v gh >/dev/null || die "gh is not installed"
command -v git >/dev/null || die "git is not installed"
gh auth status >/dev/null 2>&1 || die "gh is not logged in (gh auth login)"

say "checking the manifest (schema, hash, zip contents)"
"$TOOLS_DIR/validate-manifest.py" "$SCOOP_DIR/$APP.json" || die "the manifest does not validate"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
DIR="$WORK/bucket"

# Start from the existing bucket, or from the template (first time / dry run).
exists=0
gh repo view "$REPO" >/dev/null 2>&1 && exists=1
if [ "$exists" -eq 1 ]; then
    say "bucket $REPO exists - cloning it"
    gh repo clone "$REPO" "$DIR" -- -q
    BRANCH="$(git -C "$DIR" symbolic-ref --short HEAD)"
else
    say "bucket $REPO does not exist yet - starting from $TEMPLATE"
    git clone -q --depth 1 "https://github.com/$TEMPLATE.git" "$DIR"
    BRANCH="$(git -C "$DIR" symbolic-ref --short HEAD)"
fi

old_version=""
[ -f "$DIR/bucket/$APP.json" ] && old_version="$(manifest_version "$DIR/bucket/$APP.json")"
new_version="$(manifest_version "$SCOOP_DIR/$APP.json")"
if [ -n "$old_version" ] && [ "$old_version" != "$new_version" ] && [ "$FORCE" -eq 0 ]; then
    newest="$(printf '%s\n%s\n' "$old_version" "$new_version" | sort -V | tail -1)"
    [ "$newest" = "$new_version" ] ||
        die "the bucket already has $old_version, newer than packaging/scoop's $new_version (Excavator bumps it by itself; use --force to overwrite)"
fi

customise "$DIR" "$BRANCH"
say "style checks on the files this script writes"
style_check "$DIR" || die "style problems (above)"

if [ "$exists" -eq 1 ]; then
    git -C "$DIR" add -A
    if git -C "$DIR" diff --cached --quiet; then
        say "the bucket is already up to date ($new_version) - nothing to push"
        exit 0
    fi
    msg="$APP: $old_version -> $new_version"
    [ "$old_version" != "$new_version" ] || msg="$APP: update the bucket files"
    say "changes to push to $REPO ($BRANCH):"
    git -C "$DIR" diff --cached --stat | sed 's/^/  /'
else
    msg="Add $APP $new_version (from ${TEMPLATE##*/})"
    say "the new bucket $REPO would contain ($BRANCH):"
    (cd "$DIR" && find . -path ./.git -prune -o -type f -print | sort | sed 's/^\.\//  /')
fi
[ -z "${BUCKET_COMMIT_TRAILER:-}" ] || msg="$msg

$BUCKET_COMMIT_TRAILER"
say "commit message: ${msg%%$'\n'*}"

if [ "$DRY" -eq 1 ]; then
    say "dry run - nothing was created or pushed"
    exit 0
fi

# ---- publish ---------------------------------------------------------------------------
if [ "$exists" -eq 0 ]; then
    say "creating $REPO from $TEMPLATE"
    gh repo create "$REPO" --public --template "$TEMPLATE" \
        --description "Scoop bucket for jsonquery gui, a native desktop GUI for querying large JSON files" >/dev/null
    for _ in $(seq 1 30); do # generating from a template is asynchronous
        gh api "repos/$REPO/contents/README.md" >/dev/null 2>&1 && break
        sleep 2
    done
    gh api "repos/$REPO/contents/README.md" >/dev/null 2>&1 || die "$REPO did not get its template content within a minute"
    gh repo edit "$REPO" --add-topic scoop-bucket --add-topic scoop --add-topic windows --add-topic json >/dev/null
    rm -rf "$DIR"
    gh repo clone "$REPO" "$DIR" -- -q
    BRANCH="$(git -C "$DIR" symbolic-ref --short HEAD)"
    customise "$DIR" "$BRANCH"
    git -C "$DIR" add -A
fi
git -C "$DIR" commit -q -m "$msg"
git -C "$DIR" push -q origin "HEAD:$BRANCH"
say "pushed to https://github.com/$REPO"
echo "The bucket's CI (Actions tab) now runs Scoop's tests on the manifest; check it with:"
echo "  gh run list --repo $REPO"
echo "Users add it with:  scoop bucket add jsonquery-gui https://github.com/$REPO"
