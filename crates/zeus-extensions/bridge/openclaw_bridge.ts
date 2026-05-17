/**
 * Zeus <-> OpenClaw Bridge
 *
 * Deno script that loads an OpenClaw extension and exposes its
 * ChannelPlugin adapters via JSON-RPC over stdin/stdout.
 *
 * Usage:
 *   deno run --allow-net=api.telegram.org openclaw_bridge.ts /path/to/extension/
 *
 * Protocol:
 *   - Reads JSON-RPC requests from stdin (one per line)
 *   - Writes JSON-RPC responses to stdout (one per line)
 *   - Logs go to stderr
 */

// ---------------------------------------------------------------------------
// OpenClaw SDK Shims
// ---------------------------------------------------------------------------

// Registered channel plugin (captured during register())
let registeredPlugin: any = null;
let pluginConfig: any = {};

// Minimal shim for the OpenClaw plugin-sdk exports that extensions import.
// We only need enough to satisfy the register() call and capture the plugin.
const sdkShims = {
  emptyPluginConfigSchema: () => ({
    type: "object",
    additionalProperties: false,
    properties: {},
  }),

  getChatChannelMeta: (channelId: string) => ({
    id: channelId,
    name: channelId.charAt(0).toUpperCase() + channelId.slice(1),
    icon: channelId,
    color: "#888888",
  }),

  buildChannelConfigSchema: (schema: any) => ({
    schema: schema ?? {},
    uiHints: {},
  }),

  // Pairing helpers
  PAIRING_APPROVED_MESSAGE: "Your pairing request has been approved.",
  DEFAULT_ACCOUNT_ID: "default",

  // Config helpers (no-ops that return the config unchanged)
  applyAccountNameToChannelSection: (cfg: any) => cfg,
  deleteAccountFromConfigSection: (cfg: any) => cfg,
  setAccountEnabledInConfigSection: (cfg: any) => cfg,
  migrateBaseNameToDefaultAccount: (cfg: any) => cfg,

  // Resolve helpers (stubs)
  listTelegramAccountIds: () => ["default"],
  listTelegramDirectoryGroupsFromConfig: () => [],
  listTelegramDirectoryPeersFromConfig: () => [],
  looksLikeTelegramTargetId: () => false,
  normalizeAccountId: (id: string) => id,
  normalizeTelegramMessagingTarget: (t: string) => t,
  resolveDefaultTelegramAccountId: () => "default",
  resolveTelegramAccount: () => ({}),
  resolveTelegramGroupRequireMention: () => false,
  resolveTelegramGroupToolPolicy: () => ({}),
  collectTelegramStatusIssues: () => [],
  formatPairingApproveHint: () => "",
  telegramOnboardingAdapter: {},

  // Generic config schema types
  TelegramConfigSchema: {},
};

// Mock PluginRuntime that captures calls and bridges to Zeus
const mockRuntime: any = {
  channel: new Proxy({}, {
    get: (_target: any, channelName: string) => {
      // Return a proxy for any channel that captures method calls
      return new Proxy({}, {
        get: (_t: any, method: string) => {
          return (...args: any[]) => {
            console.error(`[bridge] runtime.channel.${channelName}.${method}() called`);
            return null;
          };
        },
      });
    },
  }),
  logger: {
    debug: (msg: string) => console.error(`[ext:debug] ${msg}`),
    info: (msg: string) => console.error(`[ext:info] ${msg}`),
    warn: (msg: string) => console.error(`[ext:warn] ${msg}`),
    error: (msg: string) => console.error(`[ext:error] ${msg}`),
  },
};

// Mock OpenClawPluginApi
const mockApi: any = {
  runtime: mockRuntime,
  registerChannel: (opts: { plugin: any }) => {
    registeredPlugin = opts.plugin;
    console.error(`[bridge] Channel plugin registered: ${opts.plugin?.id ?? "unknown"}`);
  },
  registerTool: () => {},
  registerHook: () => {},
  registerHttpRoute: () => {},
  registerService: () => {},
};

// ---------------------------------------------------------------------------
// Extension loader
// ---------------------------------------------------------------------------

async function loadExtension(extensionDir: string): Promise<void> {
  const indexPath = `${extensionDir}/index.ts`;

  try {
    // Dynamic import of the extension
    const mod = await import(`file://${indexPath}`);
    const plugin = mod.default;

    if (!plugin || typeof plugin.register !== "function") {
      console.error(`[bridge] Extension at ${indexPath} has no default export with register()`);
      Deno.exit(1);
    }

    console.error(`[bridge] Loading extension: ${plugin.id ?? plugin.name ?? "unknown"}`);
    plugin.register(mockApi);

    if (!registeredPlugin) {
      console.error("[bridge] Warning: register() did not call registerChannel()");
    }
  } catch (err) {
    console.error(`[bridge] Failed to load extension: ${err}`);
    Deno.exit(1);
  }
}

// ---------------------------------------------------------------------------
// JSON-RPC handler
// ---------------------------------------------------------------------------

interface JsonRpcRequest {
  jsonrpc: string;
  id: number;
  method: string;
  params: any;
}

interface JsonRpcResponse {
  jsonrpc: string;
  id: number;
  result?: any;
  error?: { code: number; message: string; data?: any };
}

function success(id: number, result: any): JsonRpcResponse {
  return { jsonrpc: "2.0", id, result };
}

function error(id: number, code: number, message: string): JsonRpcResponse {
  return { jsonrpc: "2.0", id, error: { code, message } };
}

// Accumulated events from callbacks
const pendingEvents: any[] = [];

async function handleRequest(req: JsonRpcRequest): Promise<JsonRpcResponse> {
  switch (req.method) {
    case "ping":
      return success(req.id, { status: "ok", plugin: registeredPlugin?.id ?? null });

    case "channel.capabilities":
      if (!registeredPlugin) {
        return error(req.id, -32001, "No channel plugin registered");
      }
      return success(req.id, registeredPlugin.capabilities ?? {});

    case "channel.meta":
      if (!registeredPlugin) {
        return error(req.id, -32001, "No channel plugin registered");
      }
      return success(req.id, registeredPlugin.meta ?? {});

    case "channel.send": {
      if (!registeredPlugin?.outbound?.send) {
        return error(req.id, -32002, "Channel plugin has no outbound.send adapter");
      }
      try {
        const result = await registeredPlugin.outbound.send(req.params, pluginConfig);
        return success(req.id, result ?? { sent: true });
      } catch (err: any) {
        return error(req.id, -32003, `send failed: ${err.message ?? err}`);
      }
    }

    case "channel.poll": {
      // Return accumulated events and clear the buffer
      const events = [...pendingEvents];
      pendingEvents.length = 0;
      return success(req.id, events);
    }

    case "channel.status": {
      if (!registeredPlugin?.status?.check) {
        return success(req.id, { connected: true, issues: [] });
      }
      try {
        const result = await registeredPlugin.status.check(pluginConfig);
        return success(req.id, result);
      } catch (err: any) {
        return success(req.id, { connected: false, error: err.message ?? String(err) });
      }
    }

    case "channel.config.set":
      pluginConfig = req.params ?? {};
      return success(req.id, { updated: true });

    case "channel.config.get":
      return success(req.id, pluginConfig);

    case "shutdown":
      console.error("[bridge] Shutdown requested");
      // Respond first, then exit
      setTimeout(() => Deno.exit(0), 100);
      return success(req.id, { status: "shutting_down" });

    default:
      return error(req.id, -32601, `Unknown method: ${req.method}`);
  }
}

// ---------------------------------------------------------------------------
// Main: stdin/stdout JSON-RPC loop
// ---------------------------------------------------------------------------

async function main() {
  const args = Deno.args;
  if (args.length < 1) {
    console.error("Usage: openclaw_bridge.ts <extension-dir>");
    Deno.exit(1);
  }

  const extensionDir = args[0];
  await loadExtension(extensionDir);

  console.error("[bridge] Ready, listening on stdin for JSON-RPC requests");

  // Read lines from stdin
  const decoder = new TextDecoder();
  const buf = new Uint8Array(65536);
  let leftover = "";

  while (true) {
    const n = await Deno.stdin.read(buf);
    if (n === null) break; // EOF

    leftover += decoder.decode(buf.subarray(0, n));
    const lines = leftover.split("\n");
    leftover = lines.pop() ?? "";

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;

      try {
        const req: JsonRpcRequest = JSON.parse(trimmed);
        const resp = await handleRequest(req);
        const out = JSON.stringify(resp) + "\n";
        await Deno.stdout.write(new TextEncoder().encode(out));
      } catch (err) {
        console.error(`[bridge] Parse error: ${err}`);
        const errResp = error(0, -32700, `Parse error: ${err}`);
        const out = JSON.stringify(errResp) + "\n";
        await Deno.stdout.write(new TextEncoder().encode(out));
      }
    }
  }
}

main().catch((err) => {
  console.error(`[bridge] Fatal: ${err}`);
  Deno.exit(1);
});
