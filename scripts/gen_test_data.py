#!/usr/bin/env python3
"""Generate a large synthetic JSON file for exercising jsonquery.

Streams output directly to disk (never builds the document in memory), so it
can produce files far larger than available RAM. Records are nested and
varied on purpose — objects, arrays, nulls, unicode strings — to stress the
tree view, and every record carries a 19-digit "snowflake" style integer id
that exceeds f64's 2^53 exact-integer range, to exercise jsonquery's
arbitrary-precision number round-tripping.

Examples:
    # ~200k records (~60-80 MB) as a single JSON array (default)
    scripts/gen_test_data.py

    # Size-targeted instead of count-targeted
    scripts/gen_test_data.py -o test-data/big.json --target-size 1GB

    # NDJSON (one JSON value per line) — exercises the app's separate
    # NDJSON/concatenated-JSON ingest path instead of a single top-level array
    scripts/gen_test_data.py -o test-data/events.ndjson --format ndjson -n 1000000
"""

import argparse
import json
import random
import string
import sys
import time
import uuid
from pathlib import Path

FIRST_NAMES = [
    "Ava", "Liam", "Noah", "Emma", "Oliver", "Sophia", "Elijah", "Mia",
    "Lucas", "Amara", "Yuki", "Sofia", "Kwame", "Priya", "Diego", "Fatima",
]
LAST_NAMES = [
    "Smith", "Johnson", "Garcia", "Chen", "Patel", "Kim", "Nguyen", "Müller",
    "Rossi", "Dubois", "Okafor", "Kowalski", "Andersson", "Silva",
]
CITIES = [
    "Auckland", "Sydney", "Tokyo", "Berlin", "Nairobi", "Toronto",
    "São Paulo", "Mumbai", "Reykjavík", "Cairo", "Oslo", "Seoul",
]
COUNTRIES = ["NZ", "AU", "JP", "DE", "KE", "CA", "BR", "IN", "IS", "EG", "NO", "KR"]
ROLES = ["admin", "editor", "viewer", "billing", "support", "owner"]
TAGS = ["beta", "internal", "vip", "trial", "flagged", "verified", "legacy", "🚀new"]
SOURCES = ["web", "mobile-ios", "mobile-android", "api", "batch-import", "cli"]
EVENTS = ["created", "updated", "login", "logout", "purchase", "refund", "error"]
NOTES = [
    "Escalated to support — pending customer reply.",
    "Auto-flagged by fraud model, manually cleared.",
    "Migrated from legacy system, some fields backfilled.",
    None,
]


def make_record(rng: random.Random, idx: int) -> dict:
    has_address = rng.random() < 0.8
    return {
        # 19-digit integer: outside f64's 2^53 exact range, like a Discord/
        # Twitter snowflake or a Postgres bigint — the number this whole
        # feature exists to round-trip correctly.
        "id": rng.randint(10**18, 10**19 - 1),
        "uuid": str(uuid.UUID(int=rng.getrandbits(128))),
        "index": idx,
        "created_at": f"2026-{rng.randint(1, 12):02d}-{rng.randint(1, 28):02d}T{rng.randint(0, 23):02d}:{rng.randint(0, 59):02d}:{rng.randint(0, 59):02d}Z",
        "active": rng.random() < 0.7,
        "score": round(rng.uniform(0, 100), 4),
        "tags": rng.sample(TAGS, rng.randint(0, 4)),
        "user": {
            "name": f"{rng.choice(FIRST_NAMES)} {rng.choice(LAST_NAMES)}",
            "email": f"user{idx}@example.test",
            "roles": rng.sample(ROLES, rng.randint(1, 3)),
            "address": {
                "city": rng.choice(CITIES),
                "country": rng.choice(COUNTRIES),
                "zip": f"{rng.randint(10000, 99999)}",
            } if has_address else None,
        },
        "metadata": {
            "source": rng.choice(SOURCES),
            "retries": rng.randint(0, 3),
            "notes": rng.choice(NOTES),
        },
        "history": [
            {
                "event": rng.choice(EVENTS),
                "at": f"2026-{rng.randint(1, 12):02d}-{rng.randint(1, 28):02d}T00:00:00Z",
            }
            for _ in range(rng.randint(0, 4))
        ],
    }


# jq queries exercising the shape make_record() above produces — kept next
# to it so the two stay in sync. Printed after generation (see main()) rather
# than duplicated in README.md.
SAMPLE_QUERIES = [
    ("Active users' name + email", ".[] | select(.active) | {name: .user.name, email: .user.email}"),
    ("Users with the admin role", '.[] | select(.user.roles | index("admin")) | .user.name'),
    ("First 10 records", ".[0:10]"),
    ("Records with more than 2 history events", ".[] | select(.history | length > 2)"),
    ("Records tagged vip", '.[] | select(.tags | index("vip"))'),
    ("Record count per country (null = no address)", "[.[] | .user.address.country] | group_by(.) | map({country: .[0], count: length})"),
    ("Exact round-tripped 19-digit id (no f64 rounding)", ".[0].id"),
]


def parse_size(text: str) -> int:
    """Parse sizes like '500MB', '2GB', '1024' (bytes) into a byte count."""
    text = text.strip().upper()
    units = {"B": 1, "KB": 1024, "MB": 1024**2, "GB": 1024**3}
    for suffix, mult in sorted(units.items(), key=lambda kv: -len(kv[0])):
        if text.endswith(suffix):
            return int(float(text[: -len(suffix)]) * mult)
    return int(text)


def human_bytes(n: int) -> str:
    size = float(n)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if size < 1024 or unit == "TB":
            return f"{size:.1f} {unit}" if unit != "B" else f"{int(size)} {unit}"
        size /= 1024
    return f"{n} B"


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate a large synthetic JSON/NDJSON file for testing jsonquery.",
    )
    parser.add_argument(
        "-o", "--output", default="test-data/large.json",
        help="Output file path (default: %(default)s)",
    )
    parser.add_argument(
        "-n", "--records", type=int, default=200_000,
        help="Number of records to generate (default: %(default)s). Ignored if --target-size is given.",
    )
    parser.add_argument(
        "--target-size",
        help="Stop once the file reaches this size instead of a fixed record count, "
             "e.g. '500MB', '2GB'. Checked periodically, so the final size overshoots slightly.",
    )
    parser.add_argument(
        "--format", choices=["json", "ndjson"], default="json",
        help="'json' (default): one big top-level array. "
             "'ndjson': one JSON value per line, no wrapping array — exercises "
             "the app's separate NDJSON/concatenated-JSON ingest path.",
    )
    parser.add_argument("--seed", type=int, default=None, help="Random seed (default: random, printed for reproducing a run).")
    parser.add_argument(
        "--pretty", action="store_true",
        help="Pretty-print each record (larger file, easier to eyeball). Off by default.",
    )
    args = parser.parse_args()

    seed = args.seed if args.seed is not None else random.SystemRandom().randrange(2**32)
    rng = random.Random(seed)

    target_bytes = parse_size(args.target_size) if args.target_size else None
    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    separators = (", ", ": ") if args.pretty else (",", ":")
    indent = 2 if args.pretty else None

    start = time.monotonic()
    count = 0
    # Re-aimed after the first checkpoint (see below) to land close to the
    # target regardless of file size; starts small so a small target doesn't
    # overshoot before the first check ever happens.
    check_every = 200

    with out_path.open("w", encoding="utf-8") as f:
        if args.format == "json":
            f.write("[\n" if args.pretty else "[")

        while True:
            if target_bytes is not None:
                if count % check_every == 0:
                    pos = f.tell()
                    if pos >= target_bytes:
                        break
                    if count > 0:
                        # Re-aim so we check roughly every ~2% of the target,
                        # rather than overshooting by a fixed record count on
                        # small targets or checking wastefully often on huge ones.
                        avg_bytes = pos / count
                        check_every = max(1, int((target_bytes * 0.02) / avg_bytes))
            elif count >= args.records:
                break

            record = make_record(rng, count)
            chunk = json.dumps(record, ensure_ascii=False, indent=indent, separators=separators)

            if args.format == "ndjson":
                f.write(chunk + "\n")
            else:
                if count > 0:
                    f.write(",\n" if args.pretty else ",")
                f.write(chunk)

            count += 1
            if count % 50_000 == 0:
                elapsed = time.monotonic() - start
                print(f"  ... {count:,} records, {human_bytes(f.tell())}, {elapsed:.1f}s", file=sys.stderr)

        if args.format == "json":
            f.write("\n]\n" if args.pretty else "]")

    elapsed = time.monotonic() - start
    size = out_path.stat().st_size
    print(f"wrote {out_path} — {count:,} records, {human_bytes(size)}, {elapsed:.1f}s (seed={seed})")

    print(f"\nSample queries to try in jsonquery against {out_path}:")
    for desc, query in SAMPLE_QUERIES:
        print(f"  # {desc}\n  {query}\n")


if __name__ == "__main__":
    main()
