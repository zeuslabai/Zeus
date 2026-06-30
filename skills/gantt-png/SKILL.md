---
name: gantt-png
description: Render a Mermaid gantt (or any Mermaid/Graphviz/PlantUML diagram) to a PNG/SVG/PDF image, server-side and headless. Use when a titan needs a gantt chart as an image file — e.g. to embed a timeline in a .docx (ImageRun) or .pdf (reportlab), or to attach a rendered chart to a message. Triggers: "gantt png", "render gantt", "gantt chart image", "mermaid to png", "diagram to image", "timeline image for the doc".
version: 1.0.0
author: zeus107
user-invocable: true
read_when:
  - render gantt
  - gantt png
  - gantt to image
  - mermaid to png
  - embed gantt in docx
  - embed gantt in pdf
  - diagram to image
---

# gantt-png — Mermaid → image (headless)

## When to Use

Trigger when you need a **gantt/diagram as an image file** (not just inline-in-the-WebUI):
- Embedding a project timeline into a generated `.docx` or `.pdf`
- Attaching a rendered chart to a Discord/Slack message
- Any Mermaid, Graphviz, or PlantUML source → PNG/SVG/PDF

**NOT for** WebUI display — that already works. The WebUI hydrates ` ```mermaid ` code
blocks client-side via `mermaid.render()` (see `apps/ZeusWeb/index.html` →
`window.zeusRenderVisuals`). This skill is the **server-side** path for when there's
no browser (office-file embed, message attachments, CI).

## How It Works

`render_gantt.py` POSTs the diagram source to **Kroki** (https://kroki.io) and writes
back the rendered bytes. No Node, no `mmdc`, no graphviz, no headless browser — one
HTTP call. (Kroki's public instance is behind Cloudflare; the script sends a real
User-Agent to avoid the 403/1010 bot block.)

## Usage

```bash
# from a file
python3 skills/gantt-png/render_gantt.py gantt.mmd -o gantt.png

# from stdin
echo "$GANTT_SRC" | python3 skills/gantt-png/render_gantt.py -o gantt.png

# inline string
python3 skills/gantt-png/render_gantt.py --code "gantt
    title Sprint
    dateFormat YYYY-MM-DD
    section Build
    task :a1, 2026-06-01, 2d" -o gantt.png

# SVG (vector) or PDF instead of PNG — inferred from the -o extension
python3 skills/gantt-png/render_gantt.py gantt.mmd -o gantt.svg

# other diagram types
python3 skills/gantt-png/render_gantt.py graph.dot --type graphviz -o graph.png

# self-hosted Kroki (offline / private)
python3 skills/gantt-png/render_gantt.py gantt.mmd -o g.png --server http://localhost:8000
```

## Office Embed (the downstream unlock)

Once office-gen is confirmed, the rendered PNG drops straight in:

```python
# DOCX — python-docx
from docx import Document
doc = Document()
doc.add_picture("gantt.png", width=Inches(6))

# PDF — reportlab
from reportlab.platypus import Image
story.append(Image("gantt.png", width=6*inch, height=2*inch))
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | success |
| 2 | bad usage / no input |
| 3 | render service error (HTTP non-200 or network failure) — stderr carries Kroki's syntax message |

## Notes

- Default render is via the public `kroki.io`. For air-gapped use, run Kroki in Docker
  (`docker run -p 8000:8000 yuzutech/kroki`) and pass `--server http://localhost:8000`.
- Output format is inferred from the `-o` extension (`.png`/`.svg`/`.pdf`), or set
  explicitly with `--format`.
