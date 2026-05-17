/**
 * iMessage Channel Extension (via BlueBubbles)
 *
 * Sends and receives iMessages via the BlueBubbles server HTTP API.
 * Requires: BlueBubbles server running on macOS with the configured password.
 *
 * BlueBubbles: https://bluebubbles.app
 */

interface BlueBubblesConfig {
  server_url?: string;
  password: string;
}

interface OutboundMessage {
  channel: string;
  target: string;
  text: string;
  media?: string[];
  reply_to?: string;
  thread_id?: string;
}

// Accumulated inbound messages
const inboundQueue: any[] = [];

// Track last message timestamp for polling
let lastMessageTs = 0;

function baseUrl(config: BlueBubblesConfig): string {
  return (config.server_url ?? "http://localhost:1234").replace(/\/$/, "");
}

async function bbFetch(
  path: string,
  config: BlueBubblesConfig,
  opts?: RequestInit,
): Promise<any> {
  const url = `${baseUrl(config)}${path}?password=${encodeURIComponent(config.password)}`;

  const resp = await fetch(url, {
    ...opts,
    headers: {
      "Content-Type": "application/json",
      ...(opts?.headers ?? {}),
    },
  });

  if (!resp.ok) {
    const errText = await resp.text();
    throw new Error(`BlueBubbles ${resp.status}: ${errText}`);
  }

  return resp.json();
}

const plugin = {
  id: "imessage",
  name: "iMessage (BlueBubbles)",

  capabilities: {
    send: true,
    receive: true,
    media: true,
    reply: true,
    thread: true,
    typing: true,
  },

  outbound: {
    async send(msg: OutboundMessage, config: BlueBubblesConfig) {
      // Send text message
      const body: any = {
        chatGuid: msg.target, // e.g. "iMessage;-;+1234567890" or group GUID
        message: msg.text,
        method: "apple-script",
      };

      // Reply to specific message
      if (msg.reply_to) {
        body.selectedMessageGuid = msg.reply_to;
      }

      const result = await bbFetch("/api/v1/message/text", config, {
        method: "POST",
        body: JSON.stringify(body),
      });

      // Send media attachments if any
      if (msg.media && msg.media.length > 0) {
        for (const mediaUrl of msg.media) {
          await bbFetch("/api/v1/message/attachment", config, {
            method: "POST",
            body: JSON.stringify({
              chatGuid: msg.target,
              attachment: mediaUrl,
              method: "apple-script",
            }),
          });
        }
      }

      return {
        sent: true,
        message_guid: result?.data?.guid ?? null,
      };
    },
  },

  inbound: {
    poll() {
      const events = [...inboundQueue];
      inboundQueue.length = 0;
      return events;
    },

    // Poll BlueBubbles for new messages
    async receive(config: BlueBubblesConfig) {
      try {
        const since = lastMessageTs || Date.now() - 60_000; // Last minute on first poll
        const result = await bbFetch(
          `/api/v1/message?after=${since}&limit=50&sort=asc`,
          config,
        );

        const messages = result?.data ?? [];
        for (const msg of messages) {
          // Skip messages we sent
          if (msg.isFromMe) continue;

          const ts = msg.dateCreated ?? msg.date ?? 0;
          if (ts > lastMessageTs) {
            lastMessageTs = ts;
          }

          inboundQueue.push({
            type: "message",
            channel: "imessage",
            from: msg.handle?.address ?? msg.handleId ?? "unknown",
            text: msg.text ?? "",
            message_id: msg.guid,
            thread_id: msg.chatGuid ?? undefined,
            media: (msg.attachments ?? []).map(
              (a: any) => `${baseUrl(config)}/api/v1/attachment/${a.guid}/download?password=${encodeURIComponent(config.password)}`,
            ),
          });
        }
      } catch {
        // Server offline or no new messages
      }
    },
  },

  status: {
    async check(config: BlueBubblesConfig) {
      try {
        const result = await bbFetch("/api/v1/server/info", config);
        if (result?.data?.os_version) {
          return {
            connected: true,
            issues: [],
            server_version: result.data.server_version,
            os_version: result.data.os_version,
          };
        }
        return { connected: false, error: "Unexpected server response" };
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
