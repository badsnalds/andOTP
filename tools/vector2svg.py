#!/usr/bin/env python3
"""Converts the legacy app's service thumbnails (Android VectorDrawable XML) to SVG.

    python3 tools/vector2svg.py

Reads legacy/android/app/src/main/res/drawable/thumb_*.xml and the enum in EntryThumbnail.java,
writes crates/andotp-gui/thumbnails/<EnumName>.svg (the names stored in andOTP backups).
"""
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEGACY = ROOT / "legacy/android/app/src/main"
OUT = ROOT / "crates/andotp-gui/thumbnails"
A = "{http://schemas.android.com/apk/res/android}"
AAPT = "{http://schemas.android.com/aapt}"


def color(value):
    """#RGB, #ARGB, #RRGGBB, #AARRGGBB -> (#rrggbb, opacity)."""
    v = value.strip().lstrip("#")
    if len(v) in (3, 4):
        v = "".join(c * 2 for c in v)
    if len(v) == 6:
        return "#" + v.lower(), 1.0
    if len(v) == 8:
        return "#" + v[2:].lower(), int(v[:2], 16) / 255
    raise ValueError(f"unsupported colour {value}")


class Converter:
    def __init__(self):
        self.defs = []
        self.counter = 0

    def new_id(self, prefix):
        self.counter += 1
        return f"{prefix}{self.counter}"

    def gradient(self, el):
        gid = self.new_id("g")
        kind = el.get(A + "type", "linear")
        stops = []
        for item in el.findall("item"):
            c, o = color(item.get(A + "color"))
            stops.append(f'<stop offset="{item.get(A + "offset")}" stop-color="{c}" stop-opacity="{o:.3f}"/>')
        if not stops:
            for key, off in (("startColor", 0), ("centerColor", 0.5), ("endColor", 1)):
                if el.get(A + key):
                    c, o = color(el.get(A + key))
                    stops.append(f'<stop offset="{off}" stop-color="{c}" stop-opacity="{o:.3f}"/>')
        if kind == "radial":
            attrs = (f'cx="{el.get(A + "centerX", "0")}" cy="{el.get(A + "centerY", "0")}" '
                     f'r="{el.get(A + "gradientRadius", "0")}"')
            tag = "radialGradient"
        else:
            attrs = (f'x1="{el.get(A + "startX", "0")}" y1="{el.get(A + "startY", "0")}" '
                     f'x2="{el.get(A + "endX", "0")}" y2="{el.get(A + "endY", "0")}"')
            tag = "linearGradient"
        self.defs.append(f'<{tag} id="{gid}" gradientUnits="userSpaceOnUse" {attrs}>{"".join(stops)}</{tag}>')
        return f"url(#{gid})"

    def paint(self, el, name):
        """Returns (paint, opacity) for fillColor / strokeColor, inline or as an aapt gradient."""
        value = el.get(A + name)
        if value:
            if value.startswith("@"):
                raise ValueError(f"resource reference {value}")
            return color(value)
        for attr in el.findall(AAPT + "attr"):
            if attr.get("name") == "android:" + name:
                g = attr.find("gradient")
                if g is not None:
                    return self.gradient(g), 1.0
        return None, 1.0

    def path(self, el):
        attrs = [f'd="{el.get(A + "pathData")}"']
        fill, fill_op = self.paint(el, "fillColor")
        attrs.append(f'fill="{fill or "none"}"')
        op = fill_op * float(el.get(A + "fillAlpha", "1"))
        if fill and op < 1:
            attrs.append(f'fill-opacity="{op:.3f}"')
        if el.get(A + "fillType", "").lower() == "evenodd":
            attrs.append('fill-rule="evenodd"')
        stroke, stroke_op = self.paint(el, "strokeColor")
        width = float(el.get(A + "strokeWidth", "0"))
        if stroke and width > 0:
            attrs.append(f'stroke="{stroke}" stroke-width="{width}"')
            op = stroke_op * float(el.get(A + "strokeAlpha", "1"))
            if op < 1:
                attrs.append(f'stroke-opacity="{op:.3f}"')
            if el.get(A + "strokeLineJoin"):
                attrs.append(f'stroke-linejoin="{el.get(A + "strokeLineJoin")}"')
            if el.get(A + "strokeLineCap"):
                attrs.append(f'stroke-linecap="{el.get(A + "strokeLineCap")}"')
        return f"<path {' '.join(attrs)}/>"

    def children(self, el):
        """Converts children; a <clip-path> clips the siblings that follow it."""
        out = []
        kids = list(el)
        for i, child in enumerate(kids):
            if child.tag == "clip-path":
                cid = self.new_id("c")
                self.defs.append(f'<clipPath id="{cid}"><path d="{child.get(A + "pathData")}"/></clipPath>')
                rest = ET.Element("group")
                rest.extend(kids[i + 1:])
                out.append(f'<g clip-path="url(#{cid})">{self.children(rest)}</g>')
                break
            if child.tag == "path":
                out.append(self.path(child))
            elif child.tag == "group":
                out.append(self.group(child))
        return "".join(out)

    def group(self, el):
        t = []
        tx, ty = float(el.get(A + "translateX", "0")), float(el.get(A + "translateY", "0"))
        px, py = float(el.get(A + "pivotX", "0")), float(el.get(A + "pivotY", "0"))
        sx, sy = float(el.get(A + "scaleX", "1")), float(el.get(A + "scaleY", "1"))
        rot = float(el.get(A + "rotation", "0"))
        # Android applies: scale and rotate around the pivot, then translate.
        if tx or ty:
            t.append(f"translate({tx} {ty})")
        if px or py:
            t.append(f"translate({px} {py})")
        if rot:
            t.append(f"rotate({rot})")
        if sx != 1 or sy != 1:
            t.append(f"scale({sx} {sy})")
        if px or py:
            t.append(f"translate({-px} {-py})")
        transform = f' transform="{" ".join(t)}"' if t else ""
        return f"<g{transform}>{self.children(el)}</g>"

    def convert(self, path):
        root = ET.parse(path).getroot()
        vw, vh = root.get(A + "viewportWidth"), root.get(A + "viewportHeight")
        body = self.children(root)
        defs = f"<defs>{''.join(self.defs)}</defs>" if self.defs else ""
        return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {vw} {vh}">{defs}{body}</svg>\n'


def main():
    java = (LEGACY / "java/org/shadowice/flocke/andotp/Utilities/EntryThumbnail.java").read_text()
    mapping = re.findall(r"^\s+(\w+)\(R\.drawable\.(thumb_\w+)\)", java, re.M)
    OUT.mkdir(parents=True, exist_ok=True)
    failed = []
    for name, drawable in mapping:
        src = LEGACY / "res/drawable" / f"{drawable}.xml"
        try:
            (OUT / f"{name}.svg").write_text(Converter().convert(src))
        except Exception as err:  # report and continue
            failed.append(f"{name}: {err}")
    print(f"converted {len(mapping) - len(failed)} of {len(mapping)} thumbnails")
    for f in failed:
        print("  failed:", f)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
