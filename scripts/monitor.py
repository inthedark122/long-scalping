#!/usr/bin/env python3
"""
Compact terminal monitor for the current release.
"""

import argparse
import json
import os
import sys
import time
import urllib.request

BASE_URL = "https://okx.tg.trading.sinvoid.me/api/indicators"
SYMBOL = "OKX#ETH-USDT-SWAP#SWAP"


def fetch_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=10) as resp:
        return json.loads(resp.read())


def snapshot(timeframe: int) -> dict:
    encoded = SYMBOL.replace("#", "%23")
    url = f"{BASE_URL}?symbol_key={encoded}&timeframe={timeframe}&limit=2"
    data = fetch_json(url).get("data", [])
    return data[0] if data else {}


def fmt(value, digits=2):
    if value is None:
        return "n/a"
    return f"{value:.{digits}f}"


def tf_line(tf: int) -> str:
    snap = snapshot(tf)
    ind = snap.get("indicators", {})
    candle = ind.get("candle", {})
    mrc = ind.get("mrc_200", {})
    volume = ind.get("volume_50", {})
    return (
        f"{tf:>4}s close={fmt(candle.get('Close'))} "
        f"ml={fmt(mrc.get('MeanLine'))} "
        f"cross={mrc.get('CurrentCross', 'n/a')} "
        f"volume_gain={fmt(volume.get('Gain'))} "
        f"buy_sell_gain={fmt(volume.get('BuySellGain'))}"
    )


def print_monitor(debug_line: str):
    os.system("clear")
    print("Structural scalping")
    print("====================")
    for tf in [60, 300]:
        print(tf_line(tf))
    print()
    print("Bot debug")
    print("---------")
    print(debug_line or "Paste last_debug here with --debug.")
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--interval", type=float, default=10.0)
    parser.add_argument("--debug", default="")
    args = parser.parse_args()

    while True:
        try:
            print_monitor(args.debug)
        except KeyboardInterrupt:
            print("\nStopped.")
            break
        except Exception as exc:
            print(f"[error] {exc}", file=sys.stderr)
        if args.once:
            break
        time.sleep(args.interval)


if __name__ == "__main__":
    main()
