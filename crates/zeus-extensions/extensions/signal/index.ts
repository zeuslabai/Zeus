/**
 * Signal Messenger Channel Extension
 *
 * Sends and receives messages via signal-cli JSON-RPC interface.
 * Requires: signal-cli running in JSON-RPC mode on the configured port.
 *
 * Start signal-cli:
 *   signal-cli -a +1234567890 jsonRpc --socket=tcp://localhost:7583
 */

interface SignalConfig {
  signal_cli_path?: string;
  account: string;
  rpc_port?: number;
}

interface OutboundMessage {
  channel: string;
  target: string;
  text: string;
  media?: string[];
  reply_to?: string;
}

let rpcId = 1;

// Accumulated inbound messages
const inboundQueue: any[] = [];

async function signalRpc(
  method: string,
  params: any,
  config: SignalConfig,
): Promise<any> {
  const port = config.rpc_port ?? 7583;
  const url = `http://localhost:${port}/api/v1/rpc`;

  const body = {
    jsonrpc: "2.0",
    id: rpcId++,
    method,
    params,
  };

  const resp = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!resp.ok) {
    const errText = await resp.text();
    throw new Error(`signal-cli RPC ${resp.status}: ${errText}`);
  }

  const result = await resp.json();
  if (result.error) {
    throw new Error(`signal-cli error: ${result.error.message}`);
  }
  return result.result;
}

const plugin = {
  id: "signal",
  name: "Signal Messenger",

  capabilities: {
    send: true,
    receive: true,
    media: true,
    reply: true,
    thread: false,
    typing: false,
  },

  outbound: {
    async send(msg: OutboundMessage, config: SignalConfig) {
      const params: any = {
        account: config.account,
        recipients: [msg.target],
        message: msg.text,
      };

      // Attach media if provided
      if (msg.media && msg.media.length > 0) {
        params.attachments = msg.media;
      }

      // Quote reply
      if (msg.reply_to) {
        params.quoteTimestamp = parseInt(msg.reply_to, 10);
      }

      const result = await signalRpc("send", params, config);
      return {
        sent: true,
        timestamp: result?.timestamp ?? null,
      };
    },
  },

  inbound: {
    poll() {
      const events = [...inboundQueue];
      inboundQueue.length = 0;
      return events;
    },

    // Receive messages via signal-cli JSON-RPC
    async receive(config: SignalConfig) {
      try {
        const messages = await signalRpc(
          "receive",
          { account: config.account, timeout: 1 },
          config,
        );

        if (!Array.isArray(messages)) return;

        for (const envelope of messages) {
          const dataMsg = envelope?.dataMessage;
          if (!dataMsg?.message) continue;

          inboundQueue.push({
            type: "message",
            channel: "signal",
            from: envelope.sourceNumber ?? envelope.source ?? "unknown",
            text: dataMsg.message,
            message_id: String(dataMsg.timestamp),
            media: (dataMsg.attachments ?? []).map(
              (a: any) => a.filename ?? a.id,
            ),
          });
        }
      } catch {
        // Timeout or no messages — expected
      }
    },
  },

  status: {
    async check(config: SignalConfig) {
      try {
        const result = await signalRpc(
          "listAccounts",
          {},
          config,
        );
        const accounts = Array.isArray(result) ? result : [];
        const found = accounts.some(
          (a: any) => a.number === config.account,
        );
        if (found) {
          return { connected: true, issues: [] };
        }
        return {
          connected: false,
          error: `Account ${config.account} not registered in signal-cli`,
        };
      } catch (err: any) {
        return { connected: false, error: err.message };
      }
    },
  },

  register(api: any) {
    api.registerChannel({ plugin });
  },
};

export default plugin;
