from __future__ import annotations

import argparse
import sys
from pathlib import Path


DEFAULT_SIZES = "16,24,32,48,64,128,256"


def parse_sizes(raw: str) -> list[tuple[int, int]]:
    sizes: list[tuple[int, int]] = []
    for part in raw.split(","):
        value = part.strip()
        if not value:
            continue
        try:
            size = int(value)
        except ValueError as exc:
            raise argparse.ArgumentTypeError(f"Invalid icon size: {value}") from exc
        if size < 8 or size > 1024:
            raise argparse.ArgumentTypeError(f"Icon size out of range: {size}")
        sizes.append((size, size))
    if not sizes:
        raise argparse.ArgumentTypeError("At least one icon size is required.")
    return sizes


def parse_color(raw: str | None) -> tuple[int, int, int, int]:
    if not raw:
        return (0, 0, 0, 0)

    value = raw.strip().lstrip("#")
    if len(value) not in (6, 8):
        raise argparse.ArgumentTypeError("Background must be #RRGGBB or #RRGGBBAA.")

    try:
        red = int(value[0:2], 16)
        green = int(value[2:4], 16)
        blue = int(value[4:6], 16)
        alpha = int(value[6:8], 16) if len(value) == 8 else 255
    except ValueError as exc:
        raise argparse.ArgumentTypeError("Background must be a hex color.") from exc

    return (red, green, blue, alpha)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Convert a WebP, PNG, or JPG image into a multi-size Windows .ico file."
    )
    parser.add_argument("input", type=Path, help="Source image path.")
    parser.add_argument(
        "output",
        type=Path,
        nargs="?",
        help="Output .ico path. Defaults to icons/<source-name>.ico.",
    )
    parser.add_argument(
        "--sizes",
        type=parse_sizes,
        default=parse_sizes(DEFAULT_SIZES),
        help=f"Comma-separated icon sizes. Default: {DEFAULT_SIZES}",
    )
    parser.add_argument(
        "--background",
        type=parse_color,
        default=parse_color(None),
        help="Optional square padding color, for example #FFFFFF. Default is transparent.",
    )
    args = parser.parse_args()

    try:
        from PIL import Image
    except ImportError:
        print(
            "Pillow is required. Install it with: python -m pip install pillow",
            file=sys.stderr,
        )
        return 1

    input_path = args.input.resolve()
    if not input_path.exists():
        print(f"Input image not found: {input_path}", file=sys.stderr)
        return 1

    output_path = (
        args.output
        if args.output is not None
        else Path.cwd() / "icons" / f"{input_path.stem}.ico"
    ).resolve()
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with Image.open(input_path) as source:
        source.seek(0)
        image = source.convert("RGBA")

    max_icon_size = max(width for width, _height in args.sizes)
    side = max(image.width, image.height, max_icon_size)
    square = Image.new("RGBA", (side, side), args.background)
    offset = ((side - image.width) // 2, (side - image.height) // 2)
    square.alpha_composite(image, offset)
    square.save(output_path, format="ICO", sizes=args.sizes)

    sizes = ", ".join(f"{width}x{height}" for width, height in args.sizes)
    print(f"Icon: {output_path}")
    print(f"Sizes: {sizes}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
