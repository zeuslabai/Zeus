# Error Handling Standardization Audit

**Branch:** `feat/error-handling-standardization`
**Status:** Audit complete — zeusmolty to implement

## Summary

The `tools.rs` file has 25+ sites using raw `Error::Tool(format!(...))` pattern. These should be converted to structured `tool_err!` macro calls for consistency and cleaner error messages.

## Error Pattern Sites (by line)

### Path Security (validate_tool_path)
- Line 663: `Error::Tool(format!("Path traversal denied: '{}'", path))`
- Line 686: `Error::Tool(format!("Access to '{}' is blocked by security policy", path))`
- Line 694: `Error::Tool(format!("Access to '{}' is blocked by security policy (symlink target)", path))`
- Line 709: `Error::Tool(format!("Access to '{}' is blocked by security policy", path))`
- Line 717: `Error::Tool(format!("Access to '{}' is blocked by security policy (symlink target)", path))`

### read_file
- Line 749: `Error::Tool(format!("Failed to read {}: {}", path, e))`
- Line 604: `Error::Tool(format!("Unknown tool: {}", name))`

### write_file
- Line 787: `Error::Tool(format!("Failed to create directories: {}", e))`
- Line 792: `Error::Tool(format!("Failed to write {}: {}", path, e))`

### edit_file
- Line 831: `Error::Tool(format!("Failed to read {}: {}", path, e))`
- Line 834: `Error::Tool(format!("Search text not found in {}", path))`
- Line 846: `Error::Tool(format!("Failed to write {}: {}", path, e))`

### list_dir
- Line 880: `Error::Tool(format!("Failed to read {}: {}", path.display(), e))`
- Line 886: `Error::Tool(format!("Failed to read entry: {}", e)))`
- Line 910: `Error::Tool(format!("Failed to read {}: {}", path.display(), e))`
- Line 916: `Error::Tool(format!("Failed to read entry: {}", e)))`

### shell
- Line 983: `Error::Tool(format!("Shell command blocked: ..."))` — multiple variants
- Line 1055: `Error::Tool(format!("..."))` — various shell validation errors
- Line 1121: `Error::Tool(format!("Command timed out after {}s", timeout_secs))`
- Line 1122: `Error::Tool(format!("Failed to execute command: {}", e))`
- Line 1142: `Error::Tool(format!("Command exited with code {}\n{}", code, result))`

### web_fetch / validate_fetch_url
- Line 1176: `Error::Tool(format!("..."))` — URL validation errors (empty path, scheme, etc.)
- Line 1248: `Error::Tool(format!("..."))` — URL blocking (private IPs, SSRF)
- Line 1261: `Error::Tool(format!("..."))` — internal host blocking
- Line 1285: `Error::Tool(format!("Invalid URL: {}", e))`
- Line 1296: `Error::Tool(format!("Failed to create client: {}", e))`
- Line 1304: `Error::Tool(format!("Unsupported method: {}", method))`
- Line 1310: `Error::Tool(format!("Request failed: {}", e))`
- Line 1322: `Error::Tool(format!("Failed to read response: {}", e))`
- Line 1385: `Error::Tool(format!("HTTP {} - {}", status, text))`

### web_search
- Line 1552: `Error::Tool(format!("Failed to create client: {}", e))`
- Line 1559: `Error::Tool(format!("Search request failed: {}", e))`
- Line 1562: `Error::Tool(format!("Search returned HTTP {}", response.status()))`
- Line 1571: `Error::Tool(format!("Failed to read search response: {}", e))`

### link_understanding
- Line 1650: `Error::Tool(format!("DNS resolution failed for '{}': {}", host, e))`
- Line 1654: `Error::Tool(format!("SSRF blocked: '{}' resolves to private IP {}", host, ip))`
- Line 1685: `Error::Tool(format!("Failed to fetch URL: {}", e))`
- Line 1828: `Error::Tool(format!("Failed to create client: {}", e))`
- Line 1834: `Error::Tool(format!("Failed to fetch URL: {}", e))`
- Line 1845: `Error::Tool(format!("HTTP {} for {}", status, url))`

### media_understanding
- Similar patterns throughout

## Proposed Solution

Define a `tool_err!` macro in `zeus_core` (or in `tools.rs`):

```rust
macro_rules! tool_err {
    ($kind:expr, $msg:expr) => {
        Error::Tool(format!("[{}] {}", $kind, $msg))
    };
    ($kind:expr, $fmt:expr, $($arg:tt)*) => {
        Error::Tool(format!("[{}] {}", $kind, format!($fmt, $($arg)*)))
    };
}
```

Categorize errors by kind:
- `Path` — path validation failures
- `IO` — file read/write failures
- `Shell` — shell command failures
- `Network` — URL/web failures
- `Tool` — unknown tool, missing args

Example conversion:
```rust
// Before
Err(Error::Tool(format!("Failed to read {}: {}", path, e)))

// After
Err(tool_err!("IO", "Failed to read {}: {}", path, e))
```

## Implementation Order

1. Define the `tool_err!` macro
2. Convert path security errors (validate_tool_path)
3. Convert file operation errors (read_file, write_file, edit_file, list_dir)
4. Convert shell errors
5. Convert network errors (web_fetch, web_search, link_understanding)
6. Convert media errors
