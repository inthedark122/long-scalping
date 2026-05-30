#!/usr/bin/env python3
"""
Live indicator monitor for ETH strategy, refreshed every N seconds.
Bot state is read via --debug argument (paste from MCP list_bots last_debug).

Usage:
    python3 monitor.py                          # refresh every 10s
    python3 monitor.py --interval 5
    python3 monitor.py --once
    python3 monitor.py --debug "sug t=445 ..."  # show last bot debug line
"""

import argparse
import json
import os
import sys
import time
import urllib.request
from datetime import datetime, timezone

BASE_IND  = "https://okx.tg.trading.sinvoid.me/api/indicators"
SYMBOL    = "OKX#ETH-USDT-SWAP#SWAP"


def fetch_json(url: str, headers: dict = None) -> dict:
    req = urllib.request.Request(url, headers=headers or {})
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read())


def fetch_indicators() -> dict:
    encoded = SYMBOL.replace("#", "%23")
    url = f"{BASE_IND}?symbol_key={encoded}&timeframe=60&limit=1"
    resp = fetch_json(url)
    return resp["data"][0]


def fmt(v, d=2):
    return f"{v:.{d}f}" if v is not None else "n/a"


def bar(value, lo, hi, width=20):
    if hi <= lo or value is None:
        return "[" + "?" * width + "]"
    pct = max(0.0, min(1.0, (value - lo) / (hi - lo)))
    filled = int(pct * width)
    return "[" + "█" * filled + "·" * (width - filled) + "]"


def print_monitor(snap: dict, bot_debug: str = ""):
    ind = snap["indicators"]
    ts_ms = snap.get("time", 0)
    ts = datetime.fromtimestamp(ts_ms / 1000, tz=timezone.utc).strftime("%H:%M:%S") if ts_ms else "?"

    candle   = ind.get("candle", {})
    mrc      = ind.get("mrc_200", {})
    ntps_ind = ind.get("ntps", {})
    vol      = ind.get("volume_50", {})
    rsi      = ind.get("rsi_14", {})

    price  = candle.get("Close")
    c_dir  = candle.get("Direction", "?")
    db     = mrc.get("DownBig")
    ub     = mrc.get("UpBig")
    ml     = mrc.get("MeanLine")
    cc     = int(mrc.get("CurrentCross", 0))
    pzone  = ((price - db) / (ub - db)) if (price and ub and db and (ub - db) > 0) else None
    rng    = (ub - db) if ub and db else None

    ntps_v   = ntps_ind.get("Value")
    ntps_min = ntps_ind.get("Min")
    ntps_max = ntps_ind.get("Max")
    bsg      = vol.get("BuySellGain")
    bsg_min  = vol.get("BuySellGainMin")
    bsg_max  = vol.get("BuySellGainMax")
    gain     = vol.get("Gain")
    gain_min = vol.get("GainMin")
    gain_max = vol.get("GainMax")
    rsi_v    = rsi.get("Value")

    # ── clear screen ──
    os.system("clear")

    W = 60
    print("═" * W)
    print(f"  ETH Monitor  {ts} UTC")
    print("═" * W)

    # Price + MRC
    dir_sym = "▲" if c_dir == "LONG" else "▼"
    print(f"  Price  {fmt(price)}  {dir_sym}   MRC cross={cc:+d}   range={fmt(rng, 1)}")
    pz_bar = bar(pzone, 0.0, 1.0, 24)
    print(f"  pzone  {fmt(pzone, 3)}  {pz_bar}")
    print()

    # MRC bands with price marker
    levels = [
        ("UpBig",    mrc.get("UpBig")),
        ("UpOuter",  mrc.get("UpOuter")),
        ("UpSmall",  mrc.get("UpSmall")),
        ("UpInner",  mrc.get("UpInner")),
        ("MeanLine", ml),
        ("DnInner",  mrc.get("DownInner")),
        ("DnSmall",  mrc.get("DownSmall")),
        ("DnOuter",  mrc.get("DownOuter")),
        ("DnBig",    mrc.get("DownBig")),
    ]
    for name, val in levels:
        marker = " ◄" if (price and val and abs(price - val) < (rng or 999) * 0.04) else ""
        prefix = "  >" if name == "MeanLine" else "   "
        print(f"{prefix} {name:<9} {fmt(val)}{marker}")
    print()

    # NTPS + BSG
    ntps_lo = min(ntps_min or 0,   (ntps_v or 0) * 0.5)
    ntps_hi = max(ntps_max or 600, (ntps_v or 0) * 1.5, ntps_lo + 1)
    bsg_lo  = min(bsg_min  or -5,  (bsg   or 0) - 0.5)
    bsg_hi  = max(bsg_max  or  5,  (bsg   or 0) + 0.5, bsg_lo + 1)
    gain_lo = min(gain_min or -3,  (gain  or 0) - 0.5)
    gain_hi = max(gain_max or  3,  (gain  or 0) + 0.5, gain_lo + 1)
    print(f"  NTPS  {fmt(ntps_v, 1):>7}  {bar(ntps_v, ntps_lo, ntps_hi, 18)}")
    print(f"  BSG   {fmt(bsg, 3):>7}  {bar(bsg, bsg_lo, bsg_hi, 18)}")
    print(f"  Gain  {fmt(gain, 3):>7}  {bar(gain, gain_lo, gain_hi, 18)}")
    print(f"  RSI   {fmt(rsi_v, 1):>7}")
    print()

    # Bot state
    print("─" * W)
    if bot_debug:
        print(f"  {bot_debug}")
    else:
        print("  Bot: run `list_bots` via MCP for live state")
    print("═" * W)
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--once",     action="store_true")
    parser.add_argument("--interval", type=float, default=10)
    parser.add_argument("--debug",    default="", help="Last bot debug string from list_bots")
    args = parser.parse_args()

    while True:
        try:
            snap = fetch_indicators()
            print_monitor(snap, args.debug)
        except KeyboardInterrupt:
            print("\nStopped.")
            break
        except Exception as e:
            print(f"[error] {e}", file=sys.stderr)

        if args.once:
            break
        try:
            time.sleep(args.interval)
        except KeyboardInterrupt:
            print("\nStopped.")
            break


if __name__ == "__main__":
    main()
