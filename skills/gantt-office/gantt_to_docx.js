#!/usr/bin/env node
/**
 * gantt_to_docx.js — embed a rendered gantt PNG into a .docx.
 *
 * Chains skills/gantt-png (render_gantt.py → PNG) with docx-js ImageRun.
 * Reads a PNG from disk, measures its dimensions from the IHDR chunk,
 * and drops it into a document at a sane on-page width (preserving aspect).
 *
 * Usage:
 *   node gantt_to_docx.js <gantt.png> <out.docx> [title]
 *
 * Exit: 0 ok / 2 usage / 3 build error
 */
const fs = require("fs");
const { Document, Packer, Paragraph, TextRun, ImageRun, HeadingLevel } = require("docx");

function pngDims(buf) {
  // PNG: 8-byte sig, then IHDR (len+type+ width(4) height(4)...)
  if (buf.length < 24 || buf.readUInt32BE(0) !== 0x89504e47) {
    throw new Error("not a PNG (bad magic)");
  }
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

async function main() {
  const [, , pngPath, outPath, title] = process.argv;
  if (!pngPath || !outPath) {
    console.error("usage: node gantt_to_docx.js <gantt.png> <out.docx> [title]");
    process.exit(2);
  }
  let png;
  try {
    png = fs.readFileSync(pngPath);
  } catch (e) {
    console.error(`cannot read ${pngPath}: ${e.message}`);
    process.exit(2);
  }
  try {
    const { width, height } = pngDims(png);
    // Fit to ~600px page width, preserve aspect ratio.
    const maxW = 600;
    const scale = width > maxW ? maxW / width : 1;
    const w = Math.round(width * scale);
    const h = Math.round(height * scale);

    const children = [];
    if (title) {
      children.push(new Paragraph({ text: title, heading: HeadingLevel.HEADING_1 }));
    }
    children.push(
      new Paragraph({
        children: [
          new ImageRun({ type: "png", data: png, transformation: { width: w, height: h } }),
        ],
      })
    );

    const doc = new Document({ sections: [{ children }] });
    const buf = await Packer.toBuffer(doc);
    fs.writeFileSync(outPath, buf);
    console.log(`ok: wrote ${buf.length} bytes -> ${outPath} (img ${w}x${h})`);
    process.exit(0);
  } catch (e) {
    console.error(`build error: ${e.message}`);
    process.exit(3);
  }
}
main();
