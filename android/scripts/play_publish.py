#!/usr/bin/env python3
"""Upload a signed bundle to Google Play, and keep the store listing in sync
with android/play/metadata.

    play_publish.py --check-listing-only              # validate the listing; no network
    play_publish.py --aab X.aab --dry-run             # show the plan; no network
    play_publish.py --aab X.aab --track internal      # upload + release notes
    play_publish.py --aab X.aab --track internal --listing --symbols X-native-debug-symbols.zip

Run it in the toolchain container through android/scripts/publish.sh, which
also hands it the credentials.

Credentials are a Play Console *service account* key (JSON): give the path with
--credentials, or set PLAY_SERVICE_ACCOUNT_JSON to the path or to the JSON
itself (CI secrets are usually the latter). See android/docs/play-store.md for
creating one. Google Play cannot create an app through the API: create the app
in Play Console and make the first upload there (android/docs/play-store.md).

A release goes to the track you name, as a *draft* unless you pass
--status completed. Drafts are reviewed and rolled out in Play Console. The
default track is `internal`, the quickest way to get a build onto your own
phone.
"""
import argparse
import json
import os
import re
import struct
import sys
from pathlib import Path

ANDROID = Path(__file__).resolve().parents[1]
METADATA = ANDROID / "play/metadata/android"
LOCALES = ["en-US"]

# Google Play's documented limits.
LIMITS = {"title": 30, "short_description": 80, "full_description": 4000, "changelog": 500}
IMAGE_RULES = {
    # name: (exact size or None, min side, max side)
    "icon": ((512, 512), None, None),
    "featureGraphic": ((1024, 500), None, None),
}
SCREENSHOT_COUNT = (2, 8)  # phone screenshots
SCREENSHOT_SIDE = (320, 3840)


def package_name():
    gradle = (ANDROID / "app/build.gradle.kts").read_text()
    return re.search(r'applicationId\s*=\s*"([^"]+)"', gradle).group(1)


def version_code():
    text = (ANDROID / "version.properties").read_text()
    return int(re.search(r"^versionCode=(\d+)", text, re.M).group(1))


def png_size(path):
    """(width, height) of a PNG, read from its header (no imaging library needed)."""
    with open(path, "rb") as f:
        head = f.read(24)
    if head[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    return struct.unpack(">II", head[16:24])


def read_text(path):
    return path.read_text(encoding="utf-8").strip()


def check_listing():
    """Problems with the store listing (empty list = fine)."""
    problems = []
    for locale in LOCALES:
        base = METADATA / locale
        for key, limit in (("title", "title"), ("short_description", "short_description"),
                           ("full_description", "full_description")):
            path = base / f"{key}.txt"
            if not path.exists():
                problems.append(f"{path.relative_to(ANDROID)} is missing")
                continue
            text = read_text(path)
            if not text:
                problems.append(f"{path.relative_to(ANDROID)} is empty")
            elif len(text) > LIMITS[limit]:
                problems.append(f"{path.relative_to(ANDROID)} is {len(text)} characters (Play allows {LIMITS[limit]})")

        changelog = base / "changelogs" / f"{version_code()}.txt"
        if not changelog.exists():
            problems.append(f"no release notes for this versionCode: {changelog.relative_to(ANDROID)}")
        elif len(read_text(changelog)) > LIMITS["changelog"]:
            problems.append(f"{changelog.relative_to(ANDROID)} is over {LIMITS['changelog']} characters")

        images = base / "images"
        for name, (exact, _, _) in IMAGE_RULES.items():
            path = images / f"{name}.png"
            if not path.exists():
                problems.append(f"{path.relative_to(ANDROID)} is missing")
            elif png_size(path) != exact:
                problems.append(f"{path.relative_to(ANDROID)} is {png_size(path)}, must be {exact}")
            elif name == "icon" and path.stat().st_size > 1024 * 1024:
                problems.append(f"{path.relative_to(ANDROID)} is over 1 MB")

        shots = sorted((images / "phoneScreenshots").glob("*.png"))
        lo, hi = SCREENSHOT_COUNT
        if not lo <= len(shots) <= hi:
            problems.append(f"{len(shots)} phone screenshots in {images.relative_to(ANDROID)}/phoneScreenshots (Play wants {lo}-{hi})")
        for shot in shots:
            w, h = png_size(shot)
            if min(w, h) < SCREENSHOT_SIDE[0] or max(w, h) > SCREENSHOT_SIDE[1] or max(w, h) > 2 * min(w, h):
                problems.append(f"{shot.relative_to(ANDROID)} is {w}x{h}: sides must be {SCREENSHOT_SIDE[0]}-{SCREENSHOT_SIDE[1]} px, long side at most twice the short")
    return problems


def load_credentials(path_or_json):
    from google.oauth2 import service_account

    scopes = ["https://www.googleapis.com/auth/androidpublisher"]
    value = path_or_json or os.environ.get("PLAY_SERVICE_ACCOUNT_JSON", "")
    if not value:
        sys.exit("no credentials: pass --credentials FILE or set PLAY_SERVICE_ACCOUNT_JSON")
    if value.lstrip().startswith("{"):
        return service_account.Credentials.from_service_account_info(json.loads(value), scopes=scopes)
    return service_account.Credentials.from_service_account_file(value, scopes=scopes)


def publish(args):
    from googleapiclient.discovery import build
    from googleapiclient.http import MediaFileUpload

    package = args.package or package_name()
    service = build("androidpublisher", "v3", credentials=load_credentials(args.credentials), cache_discovery=False)
    edits = service.edits()
    edit_id = edits.insert(packageName=package, body={}).execute()["id"]
    print(f"opened edit {edit_id} for {package}")

    print(f"uploading {args.aab} ...")
    bundle = edits.bundles().upload(
        packageName=package, editId=edit_id,
        media_body=MediaFileUpload(args.aab, mimetype="application/octet-stream", resumable=True),
    ).execute()
    code = bundle["versionCode"]
    print(f"uploaded versionCode {code}")

    if args.symbols:
        edits.deobfuscationfiles().upload(
            packageName=package, editId=edit_id, apkVersionCode=code, deobfuscationFileType="nativeCode",
            media_body=MediaFileUpload(args.symbols, mimetype="application/octet-stream"),
        ).execute()
        print("uploaded native debug symbols")

    for locale in LOCALES:
        notes_file = METADATA / locale / "changelogs" / f"{code}.txt"
        notes = read_text(notes_file) if notes_file.exists() else ""
        release = {"name": args.release_name or f"{code}", "versionCodes": [str(code)], "status": args.status}
        if notes:
            release["releaseNotes"] = [{"language": locale, "text": notes}]
    edits.tracks().update(packageName=package, editId=edit_id, track=args.track,
                          body={"track": args.track, "releases": [release]}).execute()
    print(f"release ({args.status}) on the {args.track} track")

    if args.listing:
        for locale in LOCALES:
            base = METADATA / locale
            edits.listings().update(packageName=package, editId=edit_id, language=locale, body={
                "title": read_text(base / "title.txt"),
                "shortDescription": read_text(base / "short_description.txt"),
                "fullDescription": read_text(base / "full_description.txt"),
            }).execute()
            for image_type, files in (
                ("icon", [base / "images/icon.png"]),
                ("featureGraphic", [base / "images/featureGraphic.png"]),
                ("phoneScreenshots", sorted((base / "images/phoneScreenshots").glob("*.png"))),
            ):
                edits.images().deleteall(packageName=package, editId=edit_id, language=locale, imageType=image_type).execute()
                for file in files:
                    edits.images().upload(packageName=package, editId=edit_id, language=locale, imageType=image_type,
                                          media_body=MediaFileUpload(str(file), mimetype="image/png")).execute()
            print(f"store listing synced ({locale})")

    edits.commit(packageName=package, editId=edit_id).execute()
    print("committed. Review and roll out in Play Console: https://play.google.com/console")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--aab", help="the signed bundle to upload")
    parser.add_argument("--track", default="internal", choices=["internal", "alpha", "beta", "production"])
    parser.add_argument("--status", default="draft", choices=["draft", "completed"],
                        help="draft (default): review and roll out in Play Console; completed: roll out at once")
    parser.add_argument("--listing", action="store_true", help="also sync the store listing (texts, icon, graphics, screenshots)")
    parser.add_argument("--symbols", help="native-debug-symbols.zip to upload with the bundle")
    parser.add_argument("--package", help="application id (default: read from app/build.gradle.kts)")
    parser.add_argument("--release-name", help="name of the release in Play Console (default: the versionCode)")
    parser.add_argument("--credentials", help="service account JSON key (or set PLAY_SERVICE_ACCOUNT_JSON)")
    parser.add_argument("--dry-run", action="store_true", help="validate and print the plan; make no API calls")
    parser.add_argument("--check-listing-only", action="store_true", help="only validate the store listing")
    args = parser.parse_args()

    problems = check_listing()
    for problem in problems:
        print(f"  listing: {problem}")
    if args.check_listing_only:
        sys.exit(1 if problems else 0)
    if not args.aab:
        parser.error("--aab is required (or use --check-listing-only)")
    if not Path(args.aab).is_file():
        sys.exit(f"no such bundle: {args.aab}")
    if args.listing and problems:
        sys.exit("fix the store listing first (or upload without --listing)")

    if args.dry_run:
        print(f"would upload {args.aab} to {args.package or package_name()} on the {args.track} track as a {args.status} release")
        print("would also sync the store listing" if args.listing else "would leave the store listing alone")
        return
    publish(args)


if __name__ == "__main__":
    main()
