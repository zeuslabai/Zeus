#!/usr/bin/env python3
"""
render_gantt.py — Mermaid gantt (or any mermaid diagram) → PNG/SVG.

Server-side, headless. No browser, no Node, no graphviz. Uses Kroki
(https://kroki.io) by default — a single HTTP POST of the diagram source
returns a rendered image. The output PNG drops straight into a .docx
(ImageRun) or .pdf (reportlab Image) once office-gen is confirmed.

Usage:
    # from a file
    python3 render_gantt.py gantt.mmd -o gantt.png

    # from stdin
    cat gantt.mmd | python3 render_gantt.py -o gantt.png

    # inline string
    python3 render_gantt.py --code "gantt
        title Demo
        dateFormat YYYY-MM-DD
        section A
        task1 :a1, 2026-06-01, 2d" -o out.png

    # SVG instead of PNG
    python3 render_gantt.py gantt.mmd -o gantt.svg --format svg

    # self-hosted / alternate Kroki instance
    python3 render_gantt.py gantt.mmd -o g.png --server http://localhost:8000

Exit codes:
    0  success
    2  bad usage / no input
    3  render service error (HTTP non-200 or network failure)
"""
import argparse
import sys
import urllib.request
import urllib.error

DEFAULT_SERVER = "https://kroki.io"
SUPPORTED_FORMATS = ("png", "svg", "pdf")


def render(source: str, diagram_type: str = "mermaid",
           out_format: str = "png", server: str = DEFAULT_SERVER,
           timeout: int = 30) -> bytes:
    """POST diagram source to Kroki, return rendered bytes.

    Raises RuntimeError on any non-200 / network failure.
    """
    if not source.strip():
        raise ValueError("empty diagram source")
    if out_format not in SUPPORTED_FORMATS:
        raise ValueError(f"unsupported format {out_format!r}; "
                         f"choose one of {SUPPORTED_FORMATS}")

    url = f"{server.rstrip('/')}/{diagram_type}/{out_format}"
    req = urllib.request.Request(
        url,
        data=source.encode("utf-8"),
        headers={
            "Content-Type": "text/plain",
            # Kroki's public instance sits behind Cloudflare, which 403s the
            # default urllib User-Agent (error code 1010). Send a real UA.
            "User-Agent": "Mozilla/5.0 (zeus-gantt-png/1.0)",
            "Accept": "*/*",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status != 200:
                raise RuntimeError(f"render service HTTP {resp.status}")
            return resp.read()
    except urllib.error.HTTPError as e:
        # Kroki returns the error reason in the body — surface it.
        body = ""
        try:
            body = e.read().decode("utf-8", "replace")[:500]
        except Exception:
            pass
        raise RuntimeError(f"render service HTTP {e.code}: {body}") from e
    except urllib.error.URLError as e:
        raise RuntimeError(f"network failure reaching {server}: {e.reason}") from e


def _read_source(args) -> str:
    if args.code:
        return args.code
    if args.input and args.input != "-":
        with open(args.input, "r", encoding="utf-8") as f:
            return f.read()
    # stdin
    data = sys.stdin.read()
    return data


def main(argv=None):
    p = argparse.ArgumentParser(description="Mermaid gantt → PNG/SVG via Kroki")
    p.add_argument("input", nargs="?", default="-",
                   help="path to .mmd file, or '-' for stdin (default)")
    p.add_argument("--code", help="inline diagram source (overrides input)")
    p.add_argument("-o", "--output", required=True, help="output file path")
    p.add_argument("--format", default=None,
                   choices=SUPPORTED_FORMATS,
                   help="output format (default: inferred from -o extension, else png)")
    p.add_argument("--type", default="mermaid",
                   help="diagram type (default: mermaid; also graphviz, plantuml, etc.)")
    p.add_argument("--server", default=DEFAULT_SERVER,
                   help=f"Kroki server base URL (default: {DEFAULT_SERVER})")
    p.add_argument("--timeout", type=int, default=30)
    args = p.parse_args(argv)

    # infer format from output extension if not given
    fmt = args.format
    if fmt is None:
        ext = args.output.rsplit(".", 1)[-1].lower() if "." in args.output else "png"
        fmt = ext if ext in SUPPORTED_FORMATS else "png"

    try:
        source = _read_source(args)
    except OSError as e:
        print(f"error: cannot read input: {e}", file=sys.stderr)
        return 2

    if not source.strip():
        print("error: no diagram source provided (file/stdin/--code all empty)",
              file=sys.stderr)
        return 2

    try:
        data = render(source, diagram_type=args.type, out_format=fmt,
                      server=args.server, timeout=args.timeout)
    except (RuntimeError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 3

    try:
        with open(args.output, "wb") as f:
            f.write(data)
    except OSError as e:
        print(f"error: cannot write output: {e}", file=sys.stderr)
        return 2

    print(f"ok: wrote {len(data)} bytes -> {args.output} ({fmt})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
