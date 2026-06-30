# How to Get Telegram Bot Credentials

Zeus connects to Telegram in **bot mode** using the Bot API. You'll need a bot token and optionally a chat ID.

## Step 1: Create a Bot with BotFather

1. Open Telegram and search for **@BotFather**
2. Start a chat and send `/newbot`
3. Choose a display name (e.g. "My Zeus Bot")
4. Choose a username — must end in `bot` (e.g. `my_zeus_bot`)
5. BotFather replies with your **bot token** — looks like:
   ```
   123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw
   ```
   Copy this — you'll paste it into the Zeus TUI setup wizard.

## Step 2: Get a Chat ID (optional but recommended)

The chat ID tells Zeus which group or channel to send messages to by default.

**For a private chat with yourself:**
1. Send a message to your bot
2. Open: `https://api.telegram.org/bot<YOUR_TOKEN>/getUpdates`
3. Find `"chat":{"id":...}` in the response — that's your chat ID

**For a group:**
1. Add your bot to the group
2. Send a message in the group
3. Visit the same `/getUpdates` URL — the group chat ID will be negative (e.g. `-1001234567890`)

**For a channel:**
1. Add your bot as an admin to the channel
2. The chat ID format is `@channelname` or the numeric ID from `/getUpdates`

## Step 3: Enter Credentials in Zeus

Run `zeus setup` or open the TUI and navigate to **Channels → Telegram**:

- **Bot Token** — paste the token from BotFather
- **Chat ID** — paste the numeric ID (or leave blank to receive from any chat)

## Notes

- The bot token is sensitive — treat it like a password
- If you need to reset your token: `/revoke` in BotFather
- Bot mode is simpler than user mode — Zeus does not need your personal Telegram account
- Zeus will respond to any message sent to the bot unless you restrict it with a chat ID
