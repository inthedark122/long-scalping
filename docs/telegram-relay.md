# Telegram Relay for LAB Signals

This repo includes a small local relay that can forward LAB signal messages into a Telegram channel.

## What it does

- receives a local `POST /send` request
- forwards the text to Telegram using your bot token
- lets the LAB fast-candle watcher send signal-only messages without clutter
- when a message is sent as a reply, the relay routes it into the channel's linked discussion chat so TP/SL/refresh messages become comments under the root post

## Files

- `.env` - local secrets and relay config
- `scripts/telegram_channel.py` - local relay server and one-shot sender
- `tmp/lab_fast_monitor.py` - LAB watcher that posts signal messages to the relay

## Setup

1. Create a Telegram channel.
2. Add your bot to that channel and make it an admin.
3. Put the bot token and channel id into `.env`.
4. Start the relay:

```bash
python3 scripts/telegram_channel.py --serve
```

5. Start the LAB watcher:

```bash
python3 -u tmp/lab_fast_monitor.py
```

## `.env`

Required values:

- `TELEGRAM_BOT_TOKEN`
- `TELEGRAM_CHAT_ID`

Optional values:

- `TELEGRAM_SERVER_HOST` defaults to `127.0.0.1`
- `TELEGRAM_SERVER_PORT` defaults to `8787`
- `TELEGRAM_NOTIFY_URL` defaults to `http://127.0.0.1:8787/send`
- `TELEGRAM_DISCUSSION_CHAT_ID` can be set to the linked discussion chat id; the watcher uses it to create a mirrored discussion anchor for threaded replies

## Sending a manual message

You can send a message directly:

```bash
python3 scripts/telegram_channel.py --send "LAB signal: long-reverse"
```

If `TELEGRAM_NOTIFY_URL` is set, the command sends through the local relay.
Otherwise it posts directly to Telegram.

## Message format

The LAB watcher sends:

- signal kind
- current price
- candle movement %
- fast-candle movement %
- NTPS
- 1m / 5m / 30m / 1h MRC and DP context
- root signals are posted to the channel
- refresh, TP, SL, replacement, and flip messages are sent as replies under the mirrored discussion anchor, so they appear as comments in the linked discussion chat

That keeps the message short enough for Telegram while preserving the key context.
