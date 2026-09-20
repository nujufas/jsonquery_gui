#!/usr/bin/env python3
"""Generate the Android launcher icons and the Play Store graphics from the
app's own icon (assets/icon.png), so the branding has a single source.

Run in the toolchain container (needs Pillow):
    android/scripts/gen_assets.sh

Writes (all committed; regenerate only when the icon changes):
    app/src/main/res/mipmap-*dpi/ic_launcher{,_round,_foreground,_monochrome}.png
    play/metadata/android/en-US/images/{icon,featureGraphic}.png
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
ANDROID = ROOT / "android"
RES = ANDROID / "app/src/main/res"
IMAGES = ANDROID / "play/metadata/android/en-US/images"
SOURCE = ROOT / "assets/icon.png"

BACKGROUND = (24, 22, 16)  # the icon's dark square; also @color/ic_launcher_background
DENSITIES = {"mdpi": 1.0, "hdpi": 1.5, "xhdpi": 2.0, "xxhdpi": 3.0, "xxxhdpi": 4.0}


def brace_layer(icon: Image.Image) -> Image.Image:
    """The orange braces alone, on transparency, cropped to their bounding box.

    Each pixel's opacity is how far it lies between the background colour and
    the brace colour, which keeps the anti-aliased edges smooth.
    """
    px = icon.convert("RGBA")
    # The brace colour: the most common bright opaque colour.
    counts = {}
    for r, g, b, a in px.getdata():
        if a == 255 and r > 150:
            counts[(r, g, b)] = counts.get((r, g, b), 0) + 1
    fg = max(counts, key=counts.get)
    span = [f - b for f, b in zip(fg, BACKGROUND)]
    norm = sum(s * s for s in span)

    out = Image.new("RGBA", px.size, (0, 0, 0, 0))
    src, dst = px.load(), out.load()
    for y in range(px.height):
        for x in range(px.width):
            r, g, b, a = src[x, y]
            if a == 0:
                continue
            t = sum((p - bg) * s for p, bg, s in zip((r, g, b), BACKGROUND, span)) / norm
            t = max(0.0, min(1.0, t)) * (a / 255)
            if t > 0.02:
                dst[x, y] = (*fg, round(t * 255))
    return out.crop(out.getbbox())


def fit(layer: Image.Image, box: float) -> Image.Image:
    """`layer` scaled so its longer side is `box` pixels."""
    scale = box / max(layer.size)
    return layer.resize((round(layer.width * scale), round(layer.height * scale)), Image.LANCZOS)


def centered(layer: Image.Image, canvas: int, colour=(0, 0, 0, 0)) -> Image.Image:
    out = Image.new("RGBA", (canvas, canvas), colour)
    out.alpha_composite(layer, ((canvas - layer.width) // 2, (canvas - layer.height) // 2))
    return out


def circle(image: Image.Image) -> Image.Image:
    mask = Image.new("L", image.size, 0)
    ImageDraw.Draw(mask).ellipse((0, 0, image.width - 1, image.height - 1), fill=255)
    out = Image.new("RGBA", image.size, (0, 0, 0, 0))
    out.paste(image, mask=mask)
    return out


def launcher_icons(icon: Image.Image, braces: Image.Image) -> None:
    for name, density in DENSITIES.items():
        folder = RES / f"mipmap-{name}"
        folder.mkdir(parents=True, exist_ok=True)

        # Adaptive layers are 108dp; the launcher shows the middle ~66-72dp, so
        # the braces sit inside a 44dp box, the same proportion as the icon.
        layer_px = round(108 * density)
        foreground = centered(fit(braces, 44 * density * 1.0 * (108 / 108)), layer_px)
        foreground.save(folder / "ic_launcher_foreground.png")

        mono = foreground.copy()
        alpha = mono.getchannel("A")
        mono = Image.new("RGBA", mono.size, (255, 255, 255, 0))
        mono.putalpha(alpha)
        mono.save(folder / "ic_launcher_monochrome.png")

        # Pre-Android-8 launchers: the finished icon at 48dp.
        legacy_px = round(48 * density)
        legacy = icon.resize((legacy_px, legacy_px), Image.LANCZOS)
        legacy.save(folder / "ic_launcher.png")
        flat = Image.new("RGBA", icon.size, BACKGROUND + (255,))
        flat.alpha_composite(icon)
        circle(flat.resize((legacy_px, legacy_px), Image.LANCZOS)).save(folder / "ic_launcher_round.png")


def store_icon(icon: Image.Image) -> Image.Image:
    """512x512, full-bleed (Play applies its own rounding)."""
    flat = Image.new("RGBA", (512, 512), BACKGROUND + (255,))
    flat.alpha_composite(icon.resize((512, 512), Image.LANCZOS))
    return flat.convert("RGB")


def feature_graphic(icon: Image.Image, braces: Image.Image) -> Image.Image:
    width, height = 1024, 500
    image = Image.new("RGB", (width, height), BACKGROUND)
    draw = ImageDraw.Draw(image)
    # A soft diagonal wash of the brace colour on the right.
    wash = Image.new("RGB", (width, height), BACKGROUND)
    wd = ImageDraw.Draw(wash)
    for x in range(width):
        t = max(0.0, (x - width * 0.35) / (width * 0.65))
        wd.line([(x, 0), (x, height)], fill=tuple(round(b + (o - b) * t * 0.16) for b, o in zip(BACKGROUND, (243, 120, 70))))
    image = wash
    draw = ImageDraw.Draw(image)

    mark = fit(braces, 230)
    image.paste(mark, (96 + (230 - mark.width) // 2, (height - mark.height) // 2), mark)

    title = ImageFont.load_default(size=92)
    tagline = ImageFont.load_default(size=34)
    small = ImageFont.load_default(size=26)
    x = 400
    draw.text((x, 128), "jsonquery", font=title, fill=(245, 240, 230))
    draw.text((x, 246), "Open, browse and query JSON", font=tagline, fill=(243, 120, 70))
    draw.text((x, 306), "jq  ·  JSONPath  ·  JMESPath  ·  JSON Pointer", font=small, fill=(190, 184, 170))
    return image


def main() -> None:
    icon = Image.open(SOURCE).convert("RGBA")
    braces = brace_layer(icon)
    launcher_icons(icon, braces)
    IMAGES.mkdir(parents=True, exist_ok=True)
    store_icon(icon).save(IMAGES / "icon.png")
    feature_graphic(icon, braces).save(IMAGES / "featureGraphic.png")
    print("generated launcher icons and Play graphics")


if __name__ == "__main__":
    main()
