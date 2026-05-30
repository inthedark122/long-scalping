#!/usr/bin/env python3
"""
Poll and display ETH indicator values from the LTE indicator API.

Usage:
    python3 indicators.py              # poll every 10s, all key indicators
    python3 indicators.py --once       # print once and exit
    python3 indicators.py --interval 5 # poll every 5 seconds
    python3 indicators.py --symbol OKX#BTC-USDT-SWAP#SWAP
"""

import argparse
import json
import sys
import time
import urllib.request
from datetime import datetime, timezone

BASE_URL = "https://okx.tg.trading.sinvoid.me/api/indicators"
DEFAULT_SYMBOL = "OKX#ETH-USDT-SWAP#SWAP"


def fetch(symbol_key: str, timeframe: int = 60, limit: int = 1) -> dict:
    encoded = symbol_key.replace("#", "%23")
    url = f"{BASE_URL}?symbol_key={encoded}&timeframe={timeframe}&limit={limit}"
    with urllib.request.urlopen(url, timeout=10) as resp:
        return json.loads(resp.read())


def get_snapshot(symbol_key: str) -> dict:
    resp = fetch(symbol_key, timeframe=60, limit=1)
    data = resp.get("data")
    if not data:
        raise ValueError(f"No data in response: {resp}")
    return data[0]


def fmt(val, decimals=2):
    if val is None:
        return "n/a"
    return f"{val:.{decimals}f}"


def print_snapshot(snap: dict, symbol: str = DEFAULT_SYMBOL):
    ind = snap["indicators"]
    ts_ms = snap.get("time", 0)
    ts = datetime.fromtimestamp(ts_ms / 1000, tz=timezone.utc).strftime("%H:%M:%S") if ts_ms else "?"

    candle   = ind.get("candle", {})
    mrc      = ind.get("mrc_200", {})
    ntps_ind = ind.get("ntps", {})
    vol      = ind.get("volume_50", {})
    rsi      = ind.get("rsi_14", {})
    smc      = ind.get("smc", {})

    price   = candle.get("Close")
    high    = candle.get("High")
    low     = candle.get("Low")
    c_dir   = candle.get("Direction", "?")

    db  = mrc.get("DownBig")
    do_ = mrc.get("DownOuter")
    ds  = mrc.get("DownSmall")
    di  = mrc.get("DownInner")
    ml  = mrc.get("MeanLine")
    ui  = mrc.get("UpInner")
    us  = mrc.get("UpSmall")
    uo  = mrc.get("UpOuter")
    ub  = mrc.get("UpBig")
    cc  = mrc.get("CurrentCross")
    mrc_range = (ub - db) if ub and db else None
    pzone = ((price - db) / (ub - db)) if (price and ub and db and (ub - db) > 0) else None

    ntps_val = ntps_ind.get("Value")
    ntps_min = ntps_ind.get("Min")
    ntps_max = ntps_ind.get("Max")

    bsg     = vol.get("BuySellGain")
    bsg_min = vol.get("BuySellGainMin")
    bsg_max = vol.get("BuySellGainMax")
    gain    = vol.get("Gain")
    gain_min = vol.get("GainMin")
    gain_max = vol.get("GainMax")

    rsi_val  = rsi.get("Value")
    smc_dir  = smc.get("Direction", "?")

    print(f"\n{'─'*60}")
    print(f"  {ts} UTC  |  {symbol}")
    print(f"{'─'*60}")
    print(f"  Price    {fmt(price)}  H:{fmt(high)}  L:{fmt(low)}  [{c_dir}]")
    print(f"  pzone    {fmt(pzone, 3)}  (0=DB .. 1=UB)   range={fmt(mrc_range, 1)}")
    print()
    print(f"  MRC-200 bands:")
    print(f"    UpBig    {fmt(ub)}")
    print(f"    UpOuter  {fmt(uo)}")
    print(f"    UpSmall  {fmt(us)}")
    print(f"    UpInner  {fmt(ui)}")
    print(f"  > MeanLine {fmt(ml)}  (cross={cc})")
    print(f"    DnInner  {fmt(di)}")
    print(f"    DnSmall  {fmt(ds)}")
    print(f"    DnOuter  {fmt(do_)}")
    print(f"    DnBig    {fmt(db)}")
    print()
    print(f"  NTPS     {fmt(ntps_val, 1)}  (min={fmt(ntps_min, 1)}  max={fmt(ntps_max, 1)})")
    print(f"  BSG      {fmt(bsg, 3)}  (min={fmt(bsg_min, 3)}  max={fmt(bsg_max, 3)})")
    print(f"  Gain     {fmt(gain, 3)}  (min={fmt(gain_min, 3)}  max={fmt(gain_max, 3)})")
    print(f"  RSI-14   {fmt(rsi_val, 1)}   SMC={smc_dir}")
    print(f"{'─'*60}")
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser(description="ETH indicator poller")
    parser.add_argument("--once",     action="store_true",  help="Print once and exit")
    parser.add_argument("--interval", type=float, default=10, help="Poll interval in seconds (default: 10)")
    parser.add_argument("--symbol",   default=DEFAULT_SYMBOL, help="Symbol key (default: OKX#ETH-USDT-SWAP#SWAP)")
    args = parser.parse_args()

    symbol = args.symbol

    while True:
        try:
            snap = get_snapshot(symbol)
            print_snapshot(snap, symbol)
        except KeyboardInterrupt:
            print("\nStopped.")
            break
        except Exception as e:
            print(f"[error] {e}", file=sys.stderr)

        if args.once:
            break
        try:
            time.sleep(args.interval)  # noqa
        except KeyboardInterrupt:
            print("\nStopped.")
            break


if __name__ == "__main__":
    main()
