#!/usr/bin/env python3
"""
Movement cascade research tool.

Modes:
- replay: fetch 1m history, replay at least 50 candles, simulate simple entries/exits
- live: follow the Projection websocket for N seconds and trace movement state
- both: run replay first, then live trace
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Optional

from websocket import create_connection

BASE_URL = "https://okx.tg.trading.sinvoid.me/api/indicators"
WS_URL = "wss://okx.tg.trading.sinvoid.me/ws"
DEFAULT_SYMBOL_KEY = "OKX#ETH-USDT-SWAP#SWAP"
DEFAULT_TIMEFRAME = 60


@dataclass
class CandleRow:
    time_ms: int
    open: float
    high: float
    low: float
    close: float
    dp_dir: int = 0
    zz_dir: int = 0
    dp_zee: float = 0.0
    dp_zee1: float = 0.0
    zz_buy_price: float = 0.0
    zz_sell_price: float = 0.0
    zz_break_price: float = 0.0
    volume_gain: float = 0.0
    volume_gain_min: float = 0.0
    volume_gain_max: float = 0.0
    fast_candle: Optional[dict] = None
    signal_ntps: int = 0


@dataclass
class MovementBlock:
    direction: int = 0
    start_price: float = 0.0
    high: float = 0.0
    low: float = 0.0
    candles: int = 0
    completed_sizes: list[float] = field(default_factory=list)
    avg_size: float = 0.0
    reverses: int = 0
    last_signal: str = "na"
    last_entry_reason: str = "na"
    last_exit_reason: str = "na"

    def block_size(self) -> float:
        if self.high <= 0.0 or self.low <= 0.0:
            return 0.0
        return abs(self.high - self.low)

    def stable_average(self) -> bool:
        return self.reverses >= 3 and self.avg_size > 0.0

    def phase(self) -> str:
        if self.direction == 0 or self.candles <= 1:
            return "start"
        if self.avg_size <= 0.0:
            return "building"
        size = self.block_size()
        if size >= self.avg_size * 1.3:
            return "mature"
        return "building"


@dataclass
class VirtualTrade:
    side: int
    entry: float
    tp: float
    sl: float
    entry_time_ms: int
    reason: str
    max_favorable: float = 0.0
    max_adverse: float = 0.0


@dataclass
class TradeResult:
    side: int
    entry: float
    exit: float
    reason: str
    pnl_pct: float
    mfe_pct: float
    mae_pct: float
    entry_time_ms: int
    exit_time_ms: int


@dataclass
class TradeEvent:
    kind: str
    side: str
    time_ms: int
    price: float
    reason: str
    pnl_pct: float = 0.0


def fetch_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=20) as resp:
        return json.loads(resp.read())


def fetch_history(symbol_key: str, limit: int, timeframe: int = DEFAULT_TIMEFRAME) -> list[CandleRow]:
    encoded = urllib.parse.quote(symbol_key, safe="")
    url = f"{BASE_URL}?symbol_key={encoded}&timeframe={timeframe}&limit={limit}"
    payload = fetch_json(url)
    rows = []
    for item in payload.get("data", []):
        ind = item.get("indicators", {}) or {}
        candle = ind.get("candle", {}) or {}
        dp = ind.get("dpsignal", {}) or {}
        zz = ind.get("zigzagtrend", {}) or {}
        sig = ind.get("signal", {}) or {}
        rows.append(
            CandleRow(
                time_ms=int(item.get("time", 0) or 0),
                open=float(candle.get("Open", 0.0) or 0.0),
                high=float(candle.get("High", 0.0) or 0.0),
                low=float(candle.get("Low", 0.0) or 0.0),
                close=float(candle.get("Close", 0.0) or 0.0),
                dp_dir=_dir_to_int(dp.get("Direction")),
                zz_dir=_dir_to_int(zz.get("Direction")),
                dp_zee=float(dp.get("Zee", 0.0) or 0.0),
                dp_zee1=float(dp.get("Zee1", 0.0) or 0.0),
                zz_buy_price=float(zz.get("BuyPrice", 0.0) or 0.0),
                zz_sell_price=float(zz.get("SellPrice", 0.0) or 0.0),
                zz_break_price=float(zz.get("BreakPrice", 0.0) or 0.0),
                volume_gain=float((ind.get("volume_50") or {}).get("Gain", 0.0) or 0.0),
                volume_gain_min=float((ind.get("volume_50") or {}).get("Min", 0.0) or 0.0),
                volume_gain_max=float((ind.get("volume_50") or {}).get("Max", 0.0) or 0.0),
                fast_candle=ind.get("fast_candle") or ind.get("fc"),
                signal_ntps=int(sig.get("ntps", 0) or 0),
            )
        )
    rows.sort(key=lambda row: row.time_ms)
    return rows


def _dir_to_int(value) -> int:
    if value is None:
        return 0
    if isinstance(value, str):
        up = value.upper()
        if up == "LONG":
            return 1
        if up == "SHORT":
            return -1
        return 0
    if isinstance(value, (int, float)):
        if value > 0:
            return 1
        if value < 0:
            return -1
    return 0


def fmt_time(ms: int) -> str:
    if ms <= 0:
        return "?"
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%H:%M:%S")


def choose_direction(prev: CandleRow, cur: CandleRow, last_dir: int) -> int:
    if cur.zz_dir != 0 and cur.zz_dir == cur.dp_dir:
        return cur.zz_dir
    if cur.close > prev.close:
        return 1
    if cur.close < prev.close:
        return -1
    if cur.close > cur.open:
        return 1
    if cur.close < cur.open:
        return -1
    return last_dir


def build_next_block(state: MovementBlock, direction: int, candle: CandleRow) -> None:
    if state.direction == 0:
        state.direction = direction
        state.start_price = candle.open or candle.close
        state.high = candle.high
        state.low = candle.low
        state.candles = 1
        return

    if direction == state.direction or direction == 0:
        state.high = max(state.high, candle.high)
        state.low = min(state.low, candle.low)
        state.candles += 1
        return

    completed = abs(state.high - state.low)
    if completed > 0.0:
        state.completed_sizes.append(completed)
        state.completed_sizes = state.completed_sizes[-10:]
        state.avg_size = sum(state.completed_sizes) / len(state.completed_sizes)
        state.reverses += 1

    state.direction = direction
    state.start_price = candle.open or candle.close
    state.high = candle.high
    state.low = candle.low
    state.candles = 1


def movement_signal(state: MovementBlock) -> str:
    size = state.block_size()
    if state.direction == 0 or state.candles <= 1:
        return "start"
    if state.avg_size <= 0.0:
        return "building"
    if size >= state.avg_size * 1.3 and state.stable_average():
        return "reverse-ready"
    if size <= state.avg_size * 0.8 and state.stable_average():
        return "safe-continuation"
    if size >= state.avg_size:
        return "mature"
    return "building"


def best_fill_from_sug(candle: CandleRow, side: int) -> float:
    if side > 0:
        return candle.low if candle.low > 0.0 else candle.close
    if side < 0:
        return candle.high if candle.high > 0.0 else candle.close
    return candle.close


def open_virtual_trade(
    state: MovementBlock,
    candle: CandleRow,
    side: int,
    reason: str,
    tp_mult: float,
    sl_mult: float,
) -> Optional[VirtualTrade]:
    if state.avg_size <= 0.0 or side == 0:
        return None
    entry = best_fill_from_sug(candle, side)
    if side > 0:
        tp = entry + state.avg_size * tp_mult
        sl = entry - state.avg_size * sl_mult
    else:
        tp = entry - state.avg_size * tp_mult
        sl = entry + state.avg_size * sl_mult
    return VirtualTrade(side=side, entry=entry, tp=tp, sl=sl, entry_time_ms=candle.time_ms, reason=reason)


def simulate(
    candles: list[CandleRow], tp_mult: float, sl_mult: float
) -> tuple[list[TradeResult], list[str], list[TradeEvent]]:
    state = MovementBlock()
    traces: list[str] = []
    trades: list[TradeResult] = []
    events: list[TradeEvent] = []
    active: Optional[VirtualTrade] = None

    if len(candles) < 11:
        return trades, ["not enough candles"], events

    seed = candles[:10]
    for prev, cur in zip(seed, seed[1:]):
        direction = choose_direction(prev, cur, state.direction)
        build_next_block(state, direction, cur)

    traces.append(
        f"seeded avg={state.avg_size:.4f} reverses={state.reverses} dir={state.direction}"
    )

    for prev, cur in zip(candles[9:], candles[10:]):
        direction = choose_direction(prev, cur, state.direction)
        before_dir = state.direction
        before_size = state.block_size()
        build_next_block(state, direction, cur)
        signal = movement_signal(state)
        trace = (
            f"{fmt_time(cur.time_ms)} close={cur.close:.4f} dir={state.direction:+d} "
            f"phase={state.phase()} signal={signal} size={state.block_size():.4f} "
            f"avg={state.avg_size:.4f} rev={state.reverses}"
        )
        traces.append(trace)

        if active is None:
            if signal == "reverse-ready" and before_dir != 0:
                side = -before_dir
                active = open_virtual_trade(state, cur, side, "reverse-entry", tp_mult, sl_mult)
                if active is not None:
                    events.append(
                        TradeEvent(
                            kind="enter",
                            side="LONG" if active.side > 0 else "SHORT",
                            time_ms=cur.time_ms,
                            price=active.entry,
                            reason=active.reason,
                        )
                    )
            elif signal == "safe-continuation":
                active = open_virtual_trade(state, cur, state.direction, "continuation-entry", tp_mult, sl_mult)
                if active is not None:
                    events.append(
                        TradeEvent(
                            kind="enter",
                            side="LONG" if active.side > 0 else "SHORT",
                            time_ms=cur.time_ms,
                            price=active.entry,
                            reason=active.reason,
                        )
                    )

        if active is not None:
            if active.side > 0:
                active.max_favorable = max(active.max_favorable, cur.high - active.entry)
                active.max_adverse = min(active.max_adverse, cur.low - active.entry)
                if cur.low <= active.sl:
                    exit_price = active.sl
                    trades.append(
                        finalize_trade(active, exit_price, "sl", cur.time_ms)
                    )
                    events.append(
                        TradeEvent(
                            kind="exit",
                            side="LONG",
                            time_ms=cur.time_ms,
                            price=exit_price,
                            reason="sl",
                            pnl_pct=trades[-1].pnl_pct,
                        )
                    )
                    active = None
                elif cur.high >= active.tp:
                    exit_price = active.tp
                    trades.append(
                        finalize_trade(active, exit_price, "tp", cur.time_ms)
                    )
                    events.append(
                        TradeEvent(
                            kind="exit",
                            side="LONG",
                            time_ms=cur.time_ms,
                            price=exit_price,
                            reason="tp",
                            pnl_pct=trades[-1].pnl_pct,
                        )
                    )
                    active = None
            else:
                active.max_favorable = max(active.max_favorable, active.entry - cur.low)
                active.max_adverse = min(active.max_adverse, active.entry - cur.high)
                if cur.high >= active.sl:
                    exit_price = active.sl
                    trades.append(
                        finalize_trade(active, exit_price, "sl", cur.time_ms)
                    )
                    events.append(
                        TradeEvent(
                            kind="exit",
                            side="SHORT",
                            time_ms=cur.time_ms,
                            price=exit_price,
                            reason="sl",
                            pnl_pct=trades[-1].pnl_pct,
                        )
                    )
                    active = None
                elif cur.low <= active.tp:
                    exit_price = active.tp
                    trades.append(
                        finalize_trade(active, exit_price, "tp", cur.time_ms)
                    )
                    events.append(
                        TradeEvent(
                            kind="exit",
                            side="SHORT",
                            time_ms=cur.time_ms,
                            price=exit_price,
                            reason="tp",
                            pnl_pct=trades[-1].pnl_pct,
                        )
                    )
                    active = None

    if active is not None:
        final = finalize_trade(active, candles[-1].close, "eod", candles[-1].time_ms)
        trades.append(final)
        events.append(
            TradeEvent(
                kind="exit",
                side="LONG" if active.side > 0 else "SHORT",
                time_ms=candles[-1].time_ms,
                price=candles[-1].close,
                reason="eod",
                pnl_pct=final.pnl_pct,
            )
        )

    return trades, traces, events


def finalize_trade(trade: VirtualTrade, exit_price: float, reason: str, exit_time_ms: int) -> TradeResult:
    if trade.side > 0:
        pnl_pct = ((exit_price - trade.entry) / trade.entry) * 100.0
        mfe_pct = (trade.max_favorable / trade.entry) * 100.0
        mae_pct = (abs(trade.max_adverse) / trade.entry) * 100.0
    else:
        pnl_pct = ((trade.entry - exit_price) / trade.entry) * 100.0
        mfe_pct = (trade.max_favorable / trade.entry) * 100.0
        mae_pct = (abs(trade.max_adverse) / trade.entry) * 100.0
    return TradeResult(
        side=trade.side,
        entry=trade.entry,
        exit=exit_price,
        reason=reason,
        pnl_pct=pnl_pct,
        mfe_pct=mfe_pct,
        mae_pct=mae_pct,
        entry_time_ms=trade.entry_time_ms,
        exit_time_ms=exit_time_ms,
    )


def summarize_results(results: list[TradeResult]) -> str:
    if not results:
        return "no trades"
    wins = [r for r in results if r.pnl_pct > 0.0]
    losses = [r for r in results if r.pnl_pct <= 0.0]
    avg_win = sum(r.pnl_pct for r in wins) / len(wins) if wins else 0.0
    avg_loss = sum(r.pnl_pct for r in losses) / len(losses) if losses else 0.0
    avg_mfe = sum(r.mfe_pct for r in results) / len(results)
    avg_mae = sum(r.mae_pct for r in results) / len(results)
    return (
        f"trades={len(results)} win_rate={len(wins) / len(results) * 100.0:.1f}% "
        f"avg_win={avg_win:.3f}% avg_loss={avg_loss:.3f}% avg_mfe={avg_mfe:.3f}% avg_mae={avg_mae:.3f}%"
    )


def result_rows(symbol_key: str, results: list[TradeResult]) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for r in results:
        rows.append(
            {
                "symbol_key": symbol_key,
                "kind": "trade",
                "side": "LONG" if r.side > 0 else "SHORT",
                "entry_time": fmt_time(r.entry_time_ms),
                "exit_time": fmt_time(r.exit_time_ms),
                "entry": f"{r.entry:.6f}",
                "exit": f"{r.exit:.6f}",
                "pnl_pct": f"{r.pnl_pct:.6f}",
                "mfe_pct": f"{r.mfe_pct:.6f}",
                "mae_pct": f"{r.mae_pct:.6f}",
                "reason": r.reason,
            }
        )
    if results:
        wins = [r for r in results if r.pnl_pct > 0.0]
        losses = [r for r in results if r.pnl_pct <= 0.0]
        rows.append(
            {
                "symbol_key": symbol_key,
                "kind": "summary",
                "side": "",
                "entry_time": "",
                "exit_time": "",
                "entry": "",
                "exit": "",
                "pnl_pct": "",
                "mfe_pct": "",
                "mae_pct": "",
                "reason": summarize_results(results),
                "win_rate_pct": f"{len(wins) / len(results) * 100.0:.6f}",
                "avg_win_pct": f"{(sum(r.pnl_pct for r in wins) / len(wins) if wins else 0.0):.6f}",
                "avg_loss_pct": f"{(sum(r.pnl_pct for r in losses) / len(losses) if losses else 0.0):.6f}",
            }
        )
    return rows


def print_replay(
    candles: list[CandleRow],
    results: list[TradeResult],
    traces: list[str],
    events: list[TradeEvent],
    show_events: bool,
) -> None:
    print("Movement replay")
    print("================")
    print(f"candles={len(candles)}")
    print()
    for line in traces:
        print(line)
    print()
    if show_events:
        for event in events:
            stamp = fmt_time(event.time_ms)
            if event.kind == "enter":
                print(
                    f"{stamp} ENTER {event.side} price={event.price:.4f} reason={event.reason}"
                )
            else:
                print(
                    f"{stamp} EXIT {event.side} price={event.price:.4f} pnl={event.pnl_pct:.3f}% reason={event.reason}"
                )
        print()
    print(summarize_results(results))
    if results:
        print("results:")
        for r in results:
            side = "LONG" if r.side > 0 else "SHORT"
            print(
                f"{fmt_time(r.entry_time_ms)} {side} entry={r.entry:.4f} exit={r.exit:.4f} "
                f"pnl={r.pnl_pct:.3f}% mfe={r.mfe_pct:.3f}% mae={r.mae_pct:.3f}% reason={r.reason}"
            )


def write_csv(path: str, all_rows: list[dict[str, object]]) -> None:
    if not all_rows:
        return
    fieldnames = sorted({key for row in all_rows for key in row.keys()})
    with open(path, "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(all_rows)


def live_trace(symbol: str, duration_s: int, raw: bool = False) -> None:
    ws = create_connection(WS_URL, timeout=20)
    ws.send(json.dumps({"event": "Subscribe", "param": {"event": "Projection", "symbol": symbol}}))
    state = MovementBlock()
    active: Optional[VirtualTrade] = None
    trades: list[TradeResult] = []
    events: list[TradeEvent] = []
    last_row: Optional[CandleRow] = None
    started = time.time()

    try:
        while time.time() - started < duration_s:
            msg = ws.recv()
            if not msg:
                continue
            payload = json.loads(msg)
            if payload.get("event") != "Projection":
                continue
            if raw:
                print(json.dumps(payload))
                continue
            candle = payload.get("candle", {}) or {}
            row = CandleRow(
                time_ms=int(payload.get("time", 0) or 0),
                open=float(candle.get("enter", candle.get("open", 0.0)) or 0.0),
                high=float(candle.get("max", candle.get("high", 0.0)) or 0.0),
                low=float(candle.get("min", candle.get("low", 0.0)) or 0.0),
                close=float(candle.get("exit", candle.get("close", 0.0)) or 0.0),
                signal_ntps=int((payload.get("indicator") or {}).get("ntps", 0) or 0),
            )
            last_row = row
            if state.direction == 0:
                state.direction = 1 if row.close >= row.open else -1
                state.high = row.high
                state.low = row.low
                state.candles = 1
            else:
                build_next_block(state, choose_direction(row, row, state.direction), row)
            signal = movement_signal(state)
            print(
                f"{fmt_time(row.time_ms)} close={row.close:.4f} dir={state.direction:+d} "
                f"phase={state.phase()} signal={signal} size={state.block_size():.4f} avg={state.avg_size:.4f}"
            )
            if active is None and signal == "reverse-ready" and state.direction != 0:
                active = open_virtual_trade(state, row, -state.direction, "reverse-entry", 1.0, 0.8)
                if active is not None:
                    events.append(
                        TradeEvent(
                            kind="enter",
                            side="LONG" if active.side > 0 else "SHORT",
                            time_ms=row.time_ms,
                            price=active.entry,
                            reason=active.reason,
                        )
                    )
                    print(
                        f"{fmt_time(row.time_ms)} ENTER {'LONG' if active.side > 0 else 'SHORT'} "
                        f"price={active.entry:.4f} reason={active.reason}"
                    )
            if active is not None:
                if active.side > 0 and (row.low <= active.sl or row.high >= active.tp):
                    reason = "sl" if row.low <= active.sl else "tp"
                    exit_price = active.sl if reason == "sl" else active.tp
                    pnl_pct = ((exit_price - active.entry) / active.entry) * 100.0
                    result = finalize_trade(active, exit_price, reason, row.time_ms)
                    trades.append(result)
                    events.append(
                        TradeEvent(
                            kind="exit",
                            side="LONG",
                            time_ms=row.time_ms,
                            price=exit_price,
                            reason=reason,
                            pnl_pct=pnl_pct,
                        )
                    )
                    print(
                        f"{fmt_time(row.time_ms)} EXIT LONG price={exit_price:.4f} pnl={pnl_pct:.3f}% reason={reason}"
                    )
                    active = None
                elif active.side < 0 and (row.high >= active.sl or row.low <= active.tp):
                    reason = "sl" if row.high >= active.sl else "tp"
                    exit_price = active.sl if reason == "sl" else active.tp
                    pnl_pct = ((active.entry - exit_price) / active.entry) * 100.0
                    result = finalize_trade(active, exit_price, reason, row.time_ms)
                    trades.append(result)
                    events.append(
                        TradeEvent(
                            kind="exit",
                            side="SHORT",
                            time_ms=row.time_ms,
                            price=exit_price,
                            reason=reason,
                            pnl_pct=pnl_pct,
                        )
                    )
                    print(
                        f"{fmt_time(row.time_ms)} EXIT SHORT price={exit_price:.4f} pnl={pnl_pct:.3f}% reason={reason}"
                    )
                    active = None
    except KeyboardInterrupt:
        print("\nInterrupted.")
        if active is not None and last_row is not None:
            final = finalize_trade(active, last_row.close, "interrupt", last_row.time_ms)
            trades.append(final)
            events.append(
                TradeEvent(
                    kind="exit",
                    side="LONG" if active.side > 0 else "SHORT",
                    time_ms=last_row.time_ms,
                    price=last_row.close,
                    reason="interrupt",
                    pnl_pct=final.pnl_pct,
                )
            )
            print(
                f"{fmt_time(last_row.time_ms)} EXIT {'LONG' if active.side > 0 else 'SHORT'} "
                f"price={last_row.close:.4f} pnl={final.pnl_pct:.3f}% reason=interrupt"
            )
        print(summarize_results(trades))
        if events:
            print("events:")
            for event in events:
                stamp = fmt_time(event.time_ms)
                if event.kind == "enter":
                    print(
                        f"{stamp} ENTER {event.side} price={event.price:.4f} reason={event.reason}"
                    )
                else:
                    print(
                        f"{stamp} EXIT {event.side} price={event.price:.4f} pnl={event.pnl_pct:.3f}% reason={event.reason}"
                    )
    finally:
        ws.close()


def main() -> int:
    parser = argparse.ArgumentParser(description="Movement cascade research tool")
    parser.add_argument(
        "--symbol-key",
        action="append",
        dest="symbol_keys",
        help="repeatable symbol key, e.g. OKX#ETH-USDT-SWAP#SWAP",
    )
    parser.add_argument(
        "--symbols",
        default="",
        help="comma-separated symbol keys, useful for batch runs",
    )
    parser.add_argument("--history", type=int, default=60, help="number of 1m candles to fetch")
    parser.add_argument("--mode", choices=["replay", "live", "both"], default="replay")
    parser.add_argument("--ws-seconds", type=int, default=300, help="live websocket duration in seconds")
    parser.add_argument("--tp-mult", type=float, default=1.0)
    parser.add_argument("--sl-mult", type=float, default=0.8)
    parser.add_argument("--csv", default="", help="write replay results to CSV")
    parser.add_argument("--raw", action="store_true", help="print raw websocket frames in live mode")
    args = parser.parse_args()

    symbol_keys = list(args.symbol_keys or [])
    if args.symbols:
        symbol_keys.extend([item.strip() for item in args.symbols.split(",") if item.strip()])
    if not symbol_keys:
        symbol_keys = [DEFAULT_SYMBOL_KEY]

    all_csv_rows: list[dict[str, object]] = []

    if args.mode in {"replay", "both"}:
        for symbol_key in symbol_keys:
            candles = fetch_history(symbol_key, args.history, DEFAULT_TIMEFRAME)
            if len(candles) < 50:
                print(f"{symbol_key}: need at least 50 candles, got {len(candles)}", file=sys.stderr)
                return 1
            results, traces, events = simulate(candles, args.tp_mult, args.sl_mult)
            print(f"\n=== {symbol_key} ===")
            print_replay(candles, results, traces, events, show_events=not args.csv)
            all_csv_rows.extend(result_rows(symbol_key, results))
        if args.csv:
            write_csv(args.csv, all_csv_rows)
        if args.mode == "replay":
            return 0

    if args.mode in {"live", "both"}:
        for symbol_key in symbol_keys:
            symbol = symbol_key.split("#")[1] if "#" in symbol_key else symbol_key
            live_trace(symbol, args.ws_seconds, args.raw)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
