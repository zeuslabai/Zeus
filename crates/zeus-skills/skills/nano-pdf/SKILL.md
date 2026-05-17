# nano-pdf

PDF manipulation using command-line tools (qpdf, pdftk, pdftotext).

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a PDF manipulation assistant. Help users extract text, merge, split, compress, and manipulate PDF files using command-line tools like qpdf, pdftk, and pdftotext.

## Tools

### pdf_extract_text
Extract text from a PDF file.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string",
      "description": "Input PDF path"
    },
    "pages": {
      "type": "string",
      "description": "Page range (e.g., '1-5', '1,3,5')"
    },
    "layout": {
      "type": "boolean",
      "default": false,
      "description": "Maintain original layout"
    }
  },
  "required": ["input"]
}
```

### pdf_merge
Merge multiple PDFs into one.
```json
{
  "type": "object",
  "properties": {
    "inputs": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Input PDF paths"
    },
    "output": {
      "type": "string",
      "description": "Output PDF path"
    }
  },
  "required": ["inputs", "output"]
}
```

### pdf_split
Split a PDF into multiple files.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "pages": {
      "type": "string",
      "description": "Page range to extract"
    },
    "output": {
      "type": "string"
    }
  },
  "required": ["input", "pages", "output"]
}
```

### pdf_compress
Compress a PDF to reduce file size.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "level": {
      "type": "string",
      "enum": ["screen", "ebook", "printer", "prepress"],
      "default": "ebook"
    }
  },
  "required": ["input", "output"]
}
```

### pdf_info
Get PDF metadata and info.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    }
  },
  "required": ["input"]
}
```

### pdf_rotate
Rotate pages in a PDF.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "angle": {
      "type": "integer",
      "enum": [90, 180, 270],
      "default": 90
    },
    "pages": {
      "type": "string",
      "description": "Pages to rotate (e.g., '1-5', 'all')"
    }
  },
  "required": ["input", "output"]
}
```

### pdf_encrypt
Add password protection to a PDF.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "password": {
      "type": "string"
    }
  },
  "required": ["input", "output", "password"]
}
```

## Commands

### extract_text
```bash
pdftotext {layout_flag} "{input}" -
```

### merge
```bash
qpdf --empty --pages {inputs} -- "{output}"
```

### split
```bash
qpdf "{input}" --pages . {pages} -- "{output}"
```

### compress
```bash
gs -sDEVICE=pdfwrite -dCompatibilityLevel=1.4 -dPDFSETTINGS=/{level} -dNOPAUSE -dQUIET -dBATCH -sOutputFile="{output}" "{input}"
```

### info
```bash
qpdf --show-npages "{input}" && qpdf --check "{input}"
```

### encrypt
```bash
qpdf --encrypt "{password}" "{password}" 256 -- "{input}" "{output}"
```

## Permissions
- shell
- filesystem
