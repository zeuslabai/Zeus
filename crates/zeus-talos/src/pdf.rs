//! PDF tools using macOS native Quartz/PDFKit via python3

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

/// Extract text content from a PDF file
pub struct PdfExtractTextTool;

#[async_trait]
impl TalosTool for PdfExtractTextTool {
    fn name(&self) -> &'static str {
        "pdf_extract_text"
    }
    fn description(&self) -> &'static str {
        "Extract text content from a PDF file using macOS PDFKit"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Path to the PDF file", true)
            .with_param(
                "pages",
                "string",
                "Page range to extract, e.g. \"1-5\" or \"1,3,5\" (default: all pages)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let pages = args.get("pages").and_then(|v| v.as_str());

        #[cfg(target_os = "macos")]
        {
            let safe_path = crate::sanitize_shell_arg(path);
            let page_filter = if let Some(p) = pages {
                let safe_pages = crate::sanitize_shell_arg(p);
                format!(
                    r#"
page_spec = {safe_pages}
indices = set()
for part in page_spec.split(','):
    part = part.strip()
    if '-' in part:
        start, end = part.split('-', 1)
        for i in range(int(start) - 1, int(end)):
            indices.add(i)
    else:
        indices.add(int(part) - 1)
"#
                )
            } else {
                "indices = None\n".to_string()
            };

            let script = format!(
                r#"
import sys
from Quartz import PDFDocument
from Foundation import NSURL

path = {safe_path}
url = NSURL.fileURLWithPath_(path)
doc = PDFDocument.alloc().initWithURL_(url)
if doc is None:
    print("Error: Could not open PDF file", file=sys.stderr)
    sys.exit(1)
{page_filter}
text = ""
for i in range(doc.pageCount()):
    if indices is not None and i not in indices:
        continue
    page = doc.pageAtIndex_(i)
    if page is not None:
        s = page.string()
        if s:
            text += s + "\n"
print(text)
"#
            );

            let output = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to run python3: {}", e)))?;

            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if text.trim().is_empty() {
                    Ok("No text content found in PDF".to_string())
                } else {
                    Ok(text)
                }
            } else {
                Err(Error::Tool(format!(
                    "PDF text extraction failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (path, pages);
            Ok("pdf_extract_text only available on macOS".to_string())
        }
    }
}

/// Extract specific pages from a PDF into a new PDF file
pub struct PdfExtractPagesTool;

#[async_trait]
impl TalosTool for PdfExtractPagesTool {
    fn name(&self) -> &'static str {
        "pdf_extract_pages"
    }
    fn description(&self) -> &'static str {
        "Extract specific pages from a PDF into a new PDF file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Path to the source PDF file", true)
            .with_param(
                "pages",
                "string",
                "Page range to extract, e.g. \"1-5\" or \"1,3,5\"",
                true,
            )
            .with_param("output", "string", "Output path for the new PDF file", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let pages = args
            .get("pages")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing pages".to_string()))?;

        let output_path = args
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing output".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let safe_path = crate::sanitize_shell_arg(path);
            let safe_pages = crate::sanitize_shell_arg(pages);
            let safe_output = crate::sanitize_shell_arg(output_path);

            let script = format!(
                r#"
import sys
from Quartz import PDFDocument, PDFPage
from Foundation import NSURL

path = {safe_path}
page_spec = {safe_pages}
output_path = {safe_output}

url = NSURL.fileURLWithPath_(path)
doc = PDFDocument.alloc().initWithURL_(url)
if doc is None:
    print("Error: Could not open PDF file", file=sys.stderr)
    sys.exit(1)

indices = []
for part in page_spec.split(','):
    part = part.strip()
    if '-' in part:
        start, end = part.split('-', 1)
        for i in range(int(start) - 1, int(end)):
            indices.append(i)
    else:
        indices.append(int(part) - 1)

new_doc = PDFDocument.alloc().init()
for idx in indices:
    if 0 <= idx < doc.pageCount():
        page = doc.pageAtIndex_(idx)
        new_doc.insertPage_atIndex_(page, new_doc.pageCount())

out_url = NSURL.fileURLWithPath_(output_path)
if new_doc.writeToURL_(out_url):
    print(f"Extracted {{new_doc.pageCount()}} pages to {{output_path}}")
else:
    print("Error: Failed to write output PDF", file=sys.stderr)
    sys.exit(1)
"#
            );

            let output = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to run python3: {}", e)))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                Err(Error::Tool(format!(
                    "PDF page extraction failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (path, pages, output_path);
            Ok("pdf_extract_pages only available on macOS".to_string())
        }
    }
}

/// Get metadata from a PDF file
pub struct PdfGetMetadataTool;

#[async_trait]
impl TalosTool for PdfGetMetadataTool {
    fn name(&self) -> &'static str {
        "pdf_get_metadata"
    }
    fn description(&self) -> &'static str {
        "Get metadata (title, author, page count, etc.) from a PDF file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Path to the PDF file",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let safe_path = crate::sanitize_shell_arg(path);

            let script = format!(
                r#"
import sys, json
from Quartz import PDFDocument
from Foundation import NSURL

path = {safe_path}
url = NSURL.fileURLWithPath_(path)
doc = PDFDocument.alloc().initWithURL_(url)
if doc is None:
    print("Error: Could not open PDF file", file=sys.stderr)
    sys.exit(1)

attrs = doc.documentAttributes() or {{}}
meta = {{
    "page_count": doc.pageCount(),
    "is_encrypted": doc.isEncrypted(),
    "is_locked": doc.isLocked(),
    "title": str(attrs.get("Title", "")),
    "author": str(attrs.get("Author", "")),
    "subject": str(attrs.get("Subject", "")),
    "creator": str(attrs.get("Creator", "")),
    "producer": str(attrs.get("Producer", "")),
}}

keywords = attrs.get("Keywords")
if keywords:
    meta["keywords"] = [str(k) for k in keywords]

for key in ("CreationDate", "ModDate"):
    val = attrs.get(key)
    if val:
        meta[key.lower()] = str(val)

print(json.dumps(meta, indent=2))
"#
            );

            let output = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to run python3: {}", e)))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                Err(Error::Tool(format!(
                    "PDF metadata extraction failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            Ok("pdf_get_metadata only available on macOS".to_string())
        }
    }
}

/// Merge multiple PDF files into one
pub struct PdfMergeTool;

#[async_trait]
impl TalosTool for PdfMergeTool {
    fn name(&self) -> &'static str {
        "pdf_merge"
    }
    fn description(&self) -> &'static str {
        "Merge multiple PDF files into a single PDF"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "files",
                "string",
                "Comma-separated paths to PDF files to merge",
                true,
            )
            .with_param(
                "output",
                "string",
                "Output path for the merged PDF file",
                true,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let files = args
            .get("files")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing files".to_string()))?;

        let output_path = args
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing output".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let safe_files = crate::sanitize_shell_arg(files);
            let safe_output = crate::sanitize_shell_arg(output_path);

            let script = format!(
                r#"
import sys
from Quartz import PDFDocument
from Foundation import NSURL

files_str = {safe_files}
output_path = {safe_output}

paths = [p.strip() for p in files_str.split(',') if p.strip()]
if len(paths) < 2:
    print("Error: Need at least 2 PDF files to merge", file=sys.stderr)
    sys.exit(1)

# Start with the first document
url = NSURL.fileURLWithPath_(paths[0])
merged = PDFDocument.alloc().initWithURL_(url)
if merged is None:
    print(f"Error: Could not open {{paths[0]}}", file=sys.stderr)
    sys.exit(1)

# Append pages from remaining documents
for path in paths[1:]:
    url = NSURL.fileURLWithPath_(path)
    doc = PDFDocument.alloc().initWithURL_(url)
    if doc is None:
        print(f"Error: Could not open {{path}}", file=sys.stderr)
        sys.exit(1)
    for i in range(doc.pageCount()):
        page = doc.pageAtIndex_(i)
        merged.insertPage_atIndex_(page, merged.pageCount())

out_url = NSURL.fileURLWithPath_(output_path)
if merged.writeToURL_(out_url):
    print(f"Merged {{len(paths)}} PDFs ({{merged.pageCount()}} total pages) into {{output_path}}")
else:
    print("Error: Failed to write merged PDF", file=sys.stderr)
    sys.exit(1)
"#
            );

            let output = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to run python3: {}", e)))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                Err(Error::Tool(format!(
                    "PDF merge failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (files, output_path);
            Ok("pdf_merge only available on macOS".to_string())
        }
    }
}

/// Split a PDF into individual page files
pub struct PdfSplitTool;

#[async_trait]
impl TalosTool for PdfSplitTool {
    fn name(&self) -> &'static str {
        "pdf_split"
    }
    fn description(&self) -> &'static str {
        "Split a PDF into individual page files"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Path to the PDF file to split", true)
            .with_param(
                "output_dir",
                "string",
                "Output directory for the individual page PDFs",
                true,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let output_dir = args
            .get("output_dir")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing output_dir".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let safe_path = crate::sanitize_shell_arg(path);
            let safe_output_dir = crate::sanitize_shell_arg(output_dir);

            let script = format!(
                r#"
import sys, os
from Quartz import PDFDocument
from Foundation import NSURL

path = {safe_path}
output_dir = {safe_output_dir}

url = NSURL.fileURLWithPath_(path)
doc = PDFDocument.alloc().initWithURL_(url)
if doc is None:
    print("Error: Could not open PDF file", file=sys.stderr)
    sys.exit(1)

os.makedirs(output_dir, exist_ok=True)

basename = os.path.splitext(os.path.basename(path))[0]
count = doc.pageCount()

for i in range(count):
    page = doc.pageAtIndex_(i)
    new_doc = PDFDocument.alloc().init()
    new_doc.insertPage_atIndex_(page, 0)
    out_name = f"{{basename}}_page_{{i + 1:03d}}.pdf"
    out_path = os.path.join(output_dir, out_name)
    out_url = NSURL.fileURLWithPath_(out_path)
    if not new_doc.writeToURL_(out_url):
        print(f"Error: Failed to write {{out_path}}", file=sys.stderr)
        sys.exit(1)

print(f"Split {{count}} pages into {{output_dir}}")
"#
            );

            let output = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to run python3: {}", e)))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                Err(Error::Tool(format!(
                    "PDF split failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (path, output_dir);
            Ok("pdf_split only available on macOS".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdf_extract_text_schema() {
        let tool = PdfExtractTextTool;
        assert_eq!(tool.name(), "pdf_extract_text");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        assert!(props.contains_key("pages"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
        assert!(!required.iter().any(|v| v.as_str() == Some("pages")));
    }

    #[test]
    fn test_pdf_extract_pages_schema() {
        let tool = PdfExtractPagesTool;
        assert_eq!(tool.name(), "pdf_extract_pages");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        assert!(props.contains_key("pages"));
        assert!(props.contains_key("output"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
        assert!(required.iter().any(|v| v.as_str() == Some("pages")));
        assert!(required.iter().any(|v| v.as_str() == Some("output")));
    }

    #[test]
    fn test_pdf_get_metadata_schema() {
        let tool = PdfGetMetadataTool;
        assert_eq!(tool.name(), "pdf_get_metadata");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    #[test]
    fn test_pdf_merge_schema() {
        let tool = PdfMergeTool;
        assert_eq!(tool.name(), "pdf_merge");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("files"));
        assert!(props.contains_key("output"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("files")));
        assert!(required.iter().any(|v| v.as_str() == Some("output")));
    }

    #[test]
    fn test_pdf_split_schema() {
        let tool = PdfSplitTool;
        assert_eq!(tool.name(), "pdf_split");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        assert!(props.contains_key("output_dir"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
        assert!(required.iter().any(|v| v.as_str() == Some("output_dir")));
    }
}
