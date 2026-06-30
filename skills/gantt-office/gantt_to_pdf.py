#!/usr/bin/env python3
"""gantt_to_pdf.py — embed a rendered gantt PNG into a .pdf.

Chains skills/gantt-png (render_gantt.py -> PNG) with reportlab's Image
flowable. Scales the PNG to fit the page width, preserving aspect ratio.

Usage:
    python3 gantt_to_pdf.py <gantt.png> <out.pdf> [title]

Exit: 0 ok / 2 usage / 3 build error
"""
import sys
import struct


def png_dims(path):
    with open(path, "rb") as f:
        head = f.read(24)
    if len(head) < 24 or head[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG (bad magic)")
    w, h = struct.unpack(">II", head[16:24])
    return w, h


def main():
    if len(sys.argv) < 3:
        sys.stderr.write("usage: gantt_to_pdf.py <gantt.png> <out.pdf> [title]\n")
        return 2
    png_path, out_path = sys.argv[1], sys.argv[2]
    title = sys.argv[3] if len(sys.argv) > 3 else None
    try:
        from reportlab.lib.pagesizes import letter
        from reportlab.lib.units import inch
        from reportlab.lib.styles import getSampleStyleSheet
        from reportlab.platypus import SimpleDocTemplate, Image, Paragraph, Spacer
    except ImportError as e:
        sys.stderr.write(f"missing reportlab: {e}\n")
        return 3
    try:
        w, h = png_dims(png_path)
    except (OSError, ValueError) as e:
        sys.stderr.write(f"cannot read png: {e}\n")
        return 2
    try:
        page_w, _ = letter
        avail = page_w - 2 * inch  # 1in margins each side
        scale = avail / w if w > avail else 1.0
        img_w, img_h = w * scale, h * scale

        story = []
        styles = getSampleStyleSheet()
        if title:
            story.append(Paragraph(title, styles["Title"]))
            story.append(Spacer(1, 0.25 * inch))
        story.append(Image(png_path, width=img_w, height=img_h))

        doc = SimpleDocTemplate(out_path, pagesize=letter)
        doc.build(story)
        import os
        size = os.path.getsize(out_path)
        print(f"ok: wrote {size} bytes -> {out_path} (img {img_w:.0f}x{img_h:.0f})")
        return 0
    except Exception as e:
        sys.stderr.write(f"build error: {e}\n")
        return 3


if __name__ == "__main__":
    sys.exit(main())
