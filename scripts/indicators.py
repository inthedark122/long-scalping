#!/usr/bin/env python3
"""
Print the surviving indicator set for the 1.0.0 baseline.
"""

import argparse
import json
import sys
import time
import urllib.request
from datetime import datetime, timezone

BASE_URL = "https://okx.tg.trading.sinvoid.me/api/indicators"
DEFAULT_SYMBOL = "OKX#ETH-USDT-SWAP#SWAP"
TIMEFRAMES = [60, 300, 900, 3600]


def fetch(symbol_key: str, timeframe: int) -> dict:
    encoded = symbol_key.replace("#", "%23")
    url = f"{BASE_URL}?symbol_key={encoded}&timeframe={timeframe}&limit=2"
    with urllib.request.urlopen(url, timeout=10) as resp:
        return json.loads(resp.read())


def fmt(value, digits=2):
    if value is None:
        return "n/a"
    return f"{value:.{digits}f}"


def direction(indicator: dict) -> str:
    return str(indicator.get("Direction", "NEUTRAL"))


def slope(cur: float | None, prev: float | None) -> str:
    if cur is None or prev is None:
        return "n/a"
    diff = cur - prev
    if diff > 0:
        return f"+{diff:.4f}"
    return f"{diff:.4f}"


def print_snapshot(symbol: str):
    print(f"\n{symbol}")
    print("-" * len(symbol))
    for tf in TIMEFRAMES:
        data = fetch(symbol, tf).get("data", [])
        if not data:
            print(f"{tf:>4}s  no data")
            continue
        cur = data[0].get("indicators", {})
        prev = data[1].get("indicators", {}) if len(data) > 1 else {}
        candle = cur.get("candle", {})
        mrc = cur.get("mrc_200", {})
        mrc_prev = prev.get("mrc_200", {})
        dp = cur.get("dpsignal", {})
        zz = cur.get("zigzagtrend", {})
        ts = data[0].get("time", 0)
        stamp = datetime.fromtimestamp(ts / 1000, tz=timezone.utc).strftime("%H:%M:%S") if ts else "?"
        print(
            f"{tf:>4}s  {stamp}  close={fmt(candle.get('Close'))} "
            f"dp={direction(dp):>5} zz={direction(zz):>5} "
            f"ml={fmt(mrc.get('MeanLine'))} slope={slope(mrc.get('MeanLine'), mrc_prev.get('MeanLine'))} "
            f"cross={mrc.get('CurrentCross', 'n/a')}"
        )
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser(description="Indicator reader for the 1.0.0 baseline")
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--interval", type=float, default=10.0)
    parser.add_argument("--symbol", default=DEFAULT_SYMBOL)
    args = parser.parse_args()

    while True:
        try:
            print_snapshot(args.symbol)
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
