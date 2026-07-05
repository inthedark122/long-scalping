#!/usr/bin/env python3
"""
Print live projection price and candle data without legacy feeds.
"""

import argparse
import json
import sys

from websocket import create_connection

WS_URL = "wss://okx.tg.trading.sinvoid.me/ws"


def summarize(message: dict) -> str:
    kl = message.get("klines", {}) or {}
    candle = message.get("candle", {}) or {}
    one_min = kl.get("60", {}) or {}
    return "\n".join(
        [
            f"price={message.get('price')} direction={message.get('direction')}",
            f"1m open={one_min.get('open')} high={one_min.get('high')} low={one_min.get('low')} close={one_min.get('close')}",
            f"projection candle qty={candle.get('qty')} qtym={candle.get('qtym')}",
        ]
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--symbol", default="ETH-USDT-SWAP")
    parser.add_argument("--stream", action="store_true")
    parser.add_argument("--raw", action="store_true")
    parser.add_argument("--timeout", type=float, default=15.0)
    args = parser.parse_args()

    ws = create_connection(WS_URL, timeout=args.timeout)
    ws.send(json.dumps({"event": "Subscribe", "param": {"event": "Projection", "symbol": args.symbol}}))

    received = 0
    try:
        while True:
            raw = ws.recv()
            if not raw:
                continue
            payload = json.loads(raw)
            if payload.get("event") != "Projection":
                continue
            print(json.dumps(payload) if args.raw else summarize(payload))
            received += 1
            if not args.stream:
                break
    except KeyboardInterrupt:
        pass
    finally:
        ws.close()

    if received == 0:
        print("no projection received within timeout", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
