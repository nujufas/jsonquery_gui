#!/usr/bin/env python3
"""Generate the MSIX icon assets and the Store listing logos from assets/icon.png.

    make-assets.py            writes packaging/msix/Assets/*.png and, in dist/store-listing/ (git-ignored:
                              the Store listing material is not kept in the repo), box-art-1080x1080.png
                              (1:1) and poster-720x1080.png (9:16)

The Assets are committed, so building the package needs neither Python nor Pillow; re-run this
only when assets/icon.png changes.

What the manifest references (AppxManifest.xml):
  Assets\\StoreLogo.png          package icon shown by the Store / Settings
  Assets\\Square44x44Logo.png    taskbar, title bar, app list (+ targetsize-N variants so Windows
                                never has to rescale the small ones)
  Assets\\Square150x150Logo.png  Start-menu medium tile
each with the scale-100/125/150/200/400 variants Windows picks from at different display scalings.

Needs:  pip install pillow
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent.parent / "assets" / "icon.png"
SCALES = {100: 1.0, 125: 1.25, 150: 1.5, 200: 2.0, 400: 4.0}
TARGET_SIZES = (16, 24, 32, 48, 256)

# Store listing art (Partner Center > Store listing > Store logos): opaque, light like the screenshots,
# with the app tile (the dark rounded icon) as the focus.
PAPER = (248, 246, 242)
INK = (24, 22, 16)
MUTED = (110, 104, 94)
ACCENT = (196, 84, 38)
TILE_RADIUS = 0.213  # the icon's corner radius as a fraction of its side (best circular fit to assets/icon.png)
FONTS = (
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "C:/Windows/Fonts/segoeuib.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
)


def render(src: Image.Image, size: int, fill: float = 1.0) -> Image.Image:
    """The icon scaled to `fill` of a transparent size x size canvas, centred."""
    inner = max(1, round(size * fill))
    icon = src.resize((inner, inner), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    offset = (size - inner) // 2
    canvas.paste(icon, (offset, offset), icon)
    return canvas


def font(size: int) -> ImageFont.FreeTypeFont:
    for path in FONTS:
        if Path(path).exists():
            return ImageFont.truetype(path, size)
    return ImageFont.load_default(size)  # Pillow's bundled font


def tile(src: Image.Image, size: int) -> Image.Image:
    """The app icon at `size`, with its rounded corners redrawn smooth.

    assets/icon.png has a binary alpha channel (no anti-aliasing), so enlarging it as is leaves
    stair-steps on the corners; the shape is redrawn as a supersampled rounded rectangle instead.
    """
    icon = src.resize((size, size), Image.LANCZOS)
    base = Image.new("RGBA", (size, size), INK + (255,))
    base.alpha_composite(icon)
    big = size * 4
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, big - 1, big - 1), radius=round(big * TILE_RADIUS), fill=255)
    base.putalpha(mask.resize((size, size), Image.LANCZOS))
    return base


def put_tile(canvas: Image.Image, src: Image.Image, size: int, top: int) -> None:
    """The app tile with a soft shadow, centred horizontally at `top`."""
    left = (canvas.width - size) // 2
    icon = tile(src, size)
    shadow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    shadow.paste(Image.new("RGBA", (size, size), (0, 0, 0, 60)), (left, top + size // 28), icon.getchannel("A"))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(size / 32)))
    canvas.alpha_composite(icon, (left, top))


def box_art(src: Image.Image) -> Image.Image:
    """1:1, 1080x1080: the app tile on paper, no text (it doubles as the main logo)."""
    canvas = Image.new("RGBA", (1080, 1080), PAPER + (255,))
    put_tile(canvas, src, 760, 160)
    return canvas.convert("RGB")


def poster_art(src: Image.Image) -> Image.Image:
    """9:16, 720x1080: the tile, the name, what it does and the four languages."""
    canvas = Image.new("RGBA", (720, 1080), PAPER + (255,))
    put_tile(canvas, src, 420, 190)
    draw = ImageDraw.Draw(canvas)
    centre = canvas.width // 2
    draw.text((centre, 700), "jsonquery gui", font=font(76), fill=INK, anchor="mt")
    draw.text((centre, 808), "Browse and query", font=font(38), fill=MUTED, anchor="mt")
    draw.text((centre, 858), "large JSON files", font=font(38), fill=MUTED, anchor="mt")
    draw.rounded_rectangle((centre - 40, 934, centre + 40, 940), radius=3, fill=ACCENT)
    draw.text((centre, 970), "jq  \u00b7  JSON Pointer  \u00b7  JSONPath  \u00b7  JMESPath", font=font(24), fill=ACCENT, anchor="mt")
    return canvas.convert("RGB")


def main() -> None:
    src = Image.open(SOURCE).convert("RGBA")
    assets = HERE / "Assets"
    assets.mkdir(exist_ok=True)
    for old in assets.glob("*.png"):
        old.unlink()

    written = 0

    def save(name: str, size: int, fill: float = 1.0) -> None:
        nonlocal written
        render(src, size, fill).save(assets / name, optimize=True)
        written += 1

    # (base name, size at 100% scaling, how much of the tile the icon fills)
    for base, size, fill in (("StoreLogo", 50, 1.0), ("Square44x44Logo", 44, 1.0), ("Square150x150Logo", 150, 0.86)):
        save(f"{base}.png", size, fill)  # the plain name the manifest points at
        for scale, factor in SCALES.items():
            save(f"{base}.scale-{scale}.png", round(size * factor), fill)
    for target in TARGET_SIZES:
        save(f"Square44x44Logo.targetsize-{target}.png", target)
        save(f"Square44x44Logo.targetsize-{target}_altform-unplated.png", target)

    listing = HERE.parent.parent / "dist" / "store-listing"
    listing.mkdir(parents=True, exist_ok=True)
    box_art(src).save(listing / "box-art-1080x1080.png", optimize=True)
    poster_art(src).save(listing / "poster-720x1080.png", optimize=True)
    print(f"wrote {written} assets to {assets}, and the Store logos to {listing}")


if __name__ == "__main__":
    main()
