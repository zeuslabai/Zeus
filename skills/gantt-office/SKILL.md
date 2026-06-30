---
name: gantt-office
description: "Embed a rendered Gantt chart into a .docx or .pdf. Chains skills/gantt-png (mermaid gantt -> PNG via Kroki) with docx-js ImageRun (.docx) and reportlab Image (.pdf). Use when a user wants a project schedule / timeline / Gantt chart delivered inside a Word document or PDF report, not just as a standalone image. Triggers: 'gantt in a word doc', 'timeline in the PDF report', 'schedule chart in the proposal'."
---

# gantt-office — Gantt charts embedded in Office documents

The last mile of the gantt pipeline: a rendered Gantt PNG dropped into a
real `.docx` or `.pdf`. Pairs with `skills/gantt-png` (which turns mermaid
gantt source into a PNG via Kroki — headless, no Node/mmdc/browser).

## Pipeline

```
mermaid gantt source
      │  skills/gantt-png/render_gantt.py   (Kroki HTTP POST)
      ▼
   gantt.png
      │  gantt-office                       (this skill)
      ├─► gantt_to_docx.js   → .docx  (docx-js ImageRun)
      └─► gantt_to_pdf.py    → .pdf   (reportlab Image flowable)
```

## Prerequisites (one-time, per the office-gen audit)

- **Node** (v26+ present on titan image) + `docx` npm package (docx-js).
- **Python 3** + `reportlab` (system python has it; if not, `pip install`
  into a **venv** — system pip is PEP-668 externally-managed and will reject
  a bare install).
- Network egress to `https://kroki.io` for the render step (or a self-hosted
  Kroki via `render_gantt.py --server`).

## Usage

**1. Render the gantt to PNG** (upstream skill):

```bash
python3 ../gantt-png/render_gantt.py --code "gantt
    title Project Alpha
    dateFormat YYYY-MM-DD
    section Design
    Spec   :a1, 2026-06-01, 5d
    Review :a2, after a1, 3d
    section Build
    Impl   :b1, after a2, 7d" -o gantt.png
```

**2a. Embed into a .docx:**

```bash
node gantt_to_docx.js gantt.png schedule.docx "Project Alpha — Schedule"
```

**2b. Embed into a .pdf:**

```bash
python3 gantt_to_pdf.py gantt.png schedule.pdf "Project Alpha — Schedule"
```

Both take `<png> <out> [title]`. The image is auto-scaled to fit the page
width (≈600px in docx, page-width-minus-margins in pdf) preserving aspect
ratio, read straight from the PNG IHDR chunk — no extra image lib needed for
sizing.

## Exit codes (both tools)

| code | meaning |
|------|---------|
| 0    | success |
| 2    | bad usage / unreadable or non-PNG input |
| 3    | document build error |

## Verification (how these were proven)

Three-gate, end-to-end, against a live Kroki render (584×196 PNG):

- **.docx** → valid `Microsoft Word 2007+` zip; `word/media/<hash>.png`
  present at exact byte size (11275B); `word/document.xml` references it.
  (Caught + fixed: docx-js needs explicit `type: "png"` on `ImageRun` or the
  embedded media lands as `.undefined`.)
- **.pdf** → valid `%PDF-1.4`; contains `/Image` + `/XObject` (the PNG is in
  the object graph); `sips` re-renders it to a clean 612×792 letter page.

## Notes

- Works with **any** mermaid diagram type render_gantt.py emits (flowchart,
  sequence, etc.), not only gantt — the embed step is format-agnostic, it
  just takes a PNG.
- For xlsx embed, `openpyxl`'s image support is available (see office-gen
  audit) but not wrapped here — add if a spreadsheet-embed need surfaces.
