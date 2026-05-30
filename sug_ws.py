#!/usr/bin/env python3
"""Subscribe to the LivTorgEx Projection (sug_info) WebSocket and print a compact
market-condition snapshot — including the analysis gauges trandm + asset_01 that are
NOT in the /api/indicators feed.

Usage:
  python3 sug_ws.py                 # one-shot: grab latest projection, print, exit
  python3 sug_ws.py --stream        # keep printing each projection (Ctrl-C to stop)
  python3 sug_ws.py --symbol ETH-USDT-SWAP --raw   # dump raw JSON

Source: wss://okx.tg.trading.sinvoid.me/ws
Subscribe msg: {"event":"Subscribe","param":{"event":"Projection","symbol":"<SYM>"}}
"""
import argparse
import json
import sys

from websocket import create_connection  # websocket-client (sync)

WS_URL = "wss://okx.tg.trading.sinvoid.me/ws"


def fmt_usd(x):
    try:
        x = float(x)
    except (TypeError, ValueError):
        return str(x)
    a = abs(x)
    if a >= 1e9:
        return f"{x/1e9:.2f}B"
    if a >= 1e6:
        return f"{x/1e6:.2f}M"
    if a >= 1e3:
        return f"{x/1e3:.1f}k"
    return f"{x:.1f}"


def summarize(m):
    price = m.get("price")
    ind = m.get("indicator", {}) or {}
    trandm = ind.get("trandm")
    asset_01 = ind.get("asset_01")
    ntps = ind.get("ntps")
    direction = m.get("direction")
    kl = m.get("klines", {}) or {}

    def k(tf):
        c = kl.get(tf)
        if not c:
            return "-"
        return f"{c.get('low')}-{c.get('high')}(o{c.get('open')} c{c.get('close')})"

    ob = m.get("order_book", {}) or {}
    longs = ob.get("long_levels", []) or []   # levels above price (resistance/sell walls)
    shorts = ob.get("short_levels", []) or []  # levels below price (support/buy walls)

    def top_wall(levels):
        if not levels:
            return "-"
        best = max(levels, key=lambda l: l.get("total_amount", 0))
        return f"{best.get('price')}@{fmt_usd(best.get('total_amount'))}"

    oi = m.get("oi", {}) or {}
    cdl = m.get("candle", {}) or {}

    lines = []
    osb = ""
    if isinstance(trandm, (int, float)):
        osb = " OVERSELL" if trandm <= -50 else (" OVERBUY" if trandm >= 50 else "")
    lines.append(
        f"price={price} dir={direction} | trandm={trandm:.1f}{osb} "
        f"asset_01={fmt_usd(asset_01)} (diff {fmt_usd(ind.get('asset_diff_01'))}) | ntps={ntps}"
    )
    lines.append(
        f"  px% 1h={ind.get('price_1h')} 4h={ind.get('price_4h')} "
        f"8h={ind.get('price_8h')} 24h={ind.get('price_24h')}"
    )
    lines.append(f"  kl 1m={k('60')}")
    lines.append(f"     1h={k('3600')}  4h={k('14400')}  30m={k('1800')}")
    lines.append(
        f"  OB resist(up)={top_wall(longs)}  support(dn)={top_wall(shorts)}  "
        f"buy={fmt_usd(ob.get('buy_amount'))} sell={fmt_usd(ob.get('sell_amount'))}"
    )
    lines.append(
        f"  OI={fmt_usd(oi.get('usd'))} chg={oi.get('change')}%  "
        f"net_flow(qtym_asset)={fmt_usd(cdl.get('qtym_asset'))}"
    )
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--symbol", default="ETH-USDT-SWAP")
    ap.add_argument("--stream", action="store_true")
    ap.add_argument("--raw", action="store_true")
    ap.add_argument("--timeout", type=float, default=15.0)
    args = ap.parse_args()

    ws = create_connection(WS_URL, timeout=args.timeout)
    ws.send(json.dumps({"event": "Subscribe",
                        "param": {"event": "Projection", "symbol": args.symbol}}))

    got = 0
    try:
        while True:
            raw = ws.recv()
            if not raw:
                continue
            try:
                m = json.loads(raw)
            except json.JSONDecodeError:
                continue
            # skip subscription acks / non-projection frames
            if m.get("event") != "Projection" or "indicator" not in m:
                continue
            if args.raw:
                print(json.dumps(m))
            else:
                print(summarize(m))
            got += 1
            if not args.stream:
                break
    except KeyboardInterrupt:
        pass
    finally:
        ws.close()

    if got == 0:
        print("no projection received within timeout", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
