#!/usr/bin/env python3
import argparse
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse
from urllib.error import HTTPError
from urllib.request import Request, urlopen


def load_dotenv(path: str = ".env") -> None:
    if not os.path.exists(path):
        return
    with open(path, "r", encoding="utf-8") as f:
        for raw in f:
            line = raw.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, val = line.split("=", 1)
            key = key.strip()
            val = val.strip().strip('"').strip("'")
            if key and key not in os.environ:
                os.environ[key] = val


def env(name: str, default: str = "") -> str:
    return os.environ.get(name, default)


def build_reply_parameters(
    reply_to_message_id: int | None = None,
    reply_to_chat_id: str | int | None = None,
) -> dict | None:
    if reply_to_message_id is None:
        return None
    params: dict = {"message_id": int(reply_to_message_id)}
    if reply_to_chat_id not in (None, ""):
        params["chat_id"] = str(reply_to_chat_id)
    return params


def resolve_discussion_chat_id(channel_chat_id: str) -> str | None:
    explicit = env("TELEGRAM_DISCUSSION_CHAT_ID")
    if explicit:
        return explicit

    try:
        info = telegram_get_chat(channel_chat_id)
    except Exception:
        return None

    result = info.get("result")
    if isinstance(result, dict) and result.get("linked_chat_id") not in (None, ""):
        return str(result["linked_chat_id"])
    if isinstance(result, dict):
        inner = result.get("result")
        if isinstance(inner, dict) and inner.get("linked_chat_id") not in (None, ""):
            return str(inner["linked_chat_id"])
    return None


def telegram_send(
    text: str,
    parse_mode: str = "",
    reply_to_message_id: int | None = None,
    message_thread_id: int | None = None,
    chat_id: str | None = None,
) -> dict:
    load_dotenv()
    token = env("TELEGRAM_BOT_TOKEN")
    default_chat_id = env("TELEGRAM_CHAT_ID")
    target_chat_id = chat_id or default_chat_id
    if not token or not target_chat_id:
        raise RuntimeError("TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID are required")

    payload = {
        "chat_id": target_chat_id,
        "text": text,
        "disable_web_page_preview": True,
    }
    if parse_mode:
        payload["parse_mode"] = parse_mode
    if reply_to_message_id is not None:
        payload["reply_parameters"] = build_reply_parameters(reply_to_message_id, target_chat_id)
    if message_thread_id is not None:
        payload["message_thread_id"] = message_thread_id

    url = f"https://api.telegram.org/bot{token}/sendMessage"
    req = Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urlopen(req, timeout=20) as r:
            return json.loads(r.read())
    except HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Telegram HTTP {e.code}: {raw}") from e


def forward_to_notify_url(
    text: str,
    parse_mode: str = "",
    reply_to_message_id: int | None = None,
    message_thread_id: int | None = None,
    chat_id: str | None = None,
) -> dict:
    load_dotenv()
    notify_url = env("TELEGRAM_NOTIFY_URL")
    if not notify_url:
        raise RuntimeError("TELEGRAM_NOTIFY_URL is not set")
    body = {"text": text}
    if parse_mode:
        body["parse_mode"] = parse_mode
    if reply_to_message_id is not None:
        body["reply_to_message_id"] = reply_to_message_id
    if message_thread_id is not None:
        body["message_thread_id"] = message_thread_id
    if chat_id is not None:
        body["chat_id"] = chat_id
    payload = json.dumps(body).encode("utf-8")
    req = Request(
        notify_url,
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urlopen(req, timeout=20) as r:
            raw = r.read()
            if not raw:
                return {"ok": True}
            try:
                return json.loads(raw)
            except Exception:
                return {"ok": True, "raw": raw.decode("utf-8", errors="replace")}
    except HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Relay HTTP {e.code}: {raw}") from e


def telegram_get_updates() -> dict:
    load_dotenv()
    token = env("TELEGRAM_BOT_TOKEN")
    if not token:
        raise RuntimeError("TELEGRAM_BOT_TOKEN is required")
    url = f"https://api.telegram.org/bot{token}/getUpdates"
    req = Request(url, method="GET")
    try:
        with urlopen(req, timeout=20) as r:
            return json.loads(r.read())
    except HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Telegram HTTP {e.code}: {raw}") from e


def telegram_poll_updates(offset: int | None = None, timeout: int = 20) -> dict:
    load_dotenv()
    token = env("TELEGRAM_BOT_TOKEN")
    if not token:
        raise RuntimeError("TELEGRAM_BOT_TOKEN is required")
    params = [f"timeout={timeout}"]
    if offset is not None:
        params.append(f"offset={offset}")
    query = "&".join(params)
    url = f"https://api.telegram.org/bot{token}/getUpdates?{query}"
    req = Request(url, method="GET")
    try:
        with urlopen(req, timeout=timeout + 10) as r:
            return json.loads(r.read())
    except HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Telegram HTTP {e.code}: {raw}") from e


def telegram_get_chat(chat_id: str) -> dict:
    load_dotenv()
    token = env("TELEGRAM_BOT_TOKEN")
    if not token:
        raise RuntimeError("TELEGRAM_BOT_TOKEN is required")
    url = f"https://api.telegram.org/bot{token}/getChat?chat_id={chat_id}"
    req = Request(url, method="GET")
    try:
        with urlopen(req, timeout=20) as r:
            return json.loads(r.read())
    except HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Telegram HTTP {e.code}: {raw}") from e


def normalize_chat_id(chat_id: str) -> list[str]:
    chat_id = str(chat_id).strip()
    if not chat_id:
        return []
    candidates = [chat_id]
    if chat_id.startswith("-") and not chat_id.startswith("-100"):
        candidates.append("-100" + chat_id[1:])
    if chat_id.startswith("-100"):
        candidates.append("-" + chat_id[4:])
    # de-dup, preserve order
    out = []
    for c in candidates:
        if c not in out:
            out.append(c)
    return out


def resolve_chat_ids(updates: dict) -> dict:
    results = updates.get("result", []) or []
    ids = []
    latest = None
    latest_update_id = None

    for item in results:
        if not isinstance(item, dict):
            continue
        update_id = item.get("update_id")
        if latest_update_id is None or (isinstance(update_id, int) and update_id >= latest_update_id):
            latest_update_id = update_id
            latest = item
        for key in ("message", "channel_post", "edited_message", "edited_channel_post"):
            obj = item.get(key)
            if isinstance(obj, dict):
                chat = obj.get("chat")
                if isinstance(chat, dict) and "id" in chat:
                    ids.append(
                        {
                            "chat_id": chat.get("id"),
                            "type": chat.get("type"),
                            "title": chat.get("title"),
                            "username": chat.get("username"),
                            "message_id": obj.get("message_id"),
                            "date": obj.get("date"),
                            "source": key,
                        }
                    )
    unique = []
    seen = set()
    for item in ids:
        chat_id = item.get("chat_id")
        if chat_id in seen:
            continue
        seen.add(chat_id)
        unique.append(item)

    latest_info = None
    if isinstance(latest, dict):
        for key in ("message", "channel_post", "edited_message", "edited_channel_post"):
            obj = latest.get(key)
            if isinstance(obj, dict):
                chat = obj.get("chat") or {}
                latest_info = {
                    "update_id": latest.get("update_id"),
                    "source": key,
                    "chat_id": chat.get("id"),
                    "chat_type": chat.get("type"),
                    "chat_title": chat.get("title"),
                    "chat_username": chat.get("username"),
                    "message_id": obj.get("message_id"),
                    "date": obj.get("date"),
                    "text": obj.get("text"),
                }
                break

    return {"ok": True, "chat_ids": unique, "latest": latest_info}


def resolve_env_chat() -> dict:
    load_dotenv()
    chat_id = env("TELEGRAM_CHAT_ID")
    if not chat_id:
        raise RuntimeError("TELEGRAM_CHAT_ID is required")
    tried = []
    for candidate in normalize_chat_id(chat_id):
        try:
            info = telegram_get_chat(candidate)
            if info.get("ok"):
                return {"ok": True, "candidate": candidate, "result": info}
            tried.append({"candidate": candidate, "result": info})
        except Exception as e:
            tried.append({"candidate": candidate, "error": str(e)})
    return {"ok": False, "tried": tried}


def telegram_send_auto(
    text: str,
    parse_mode: str = "",
    reply_to_message_id: int | None = None,
    message_thread_id: int | None = None,
    chat_id: str | None = None,
) -> dict:
    load_dotenv()
    token = env("TELEGRAM_BOT_TOKEN")
    default_chat_id = env("TELEGRAM_CHAT_ID")
    target_chat_id = chat_id or default_chat_id
    if not token or not target_chat_id:
        raise RuntimeError("TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID are required")
    tried = []
    reply_parameters = build_reply_parameters(reply_to_message_id, target_chat_id) if reply_to_message_id is not None else None
    last_err = None
    for candidate in normalize_chat_id(target_chat_id):
        payload = {
            "chat_id": candidate,
            "text": text,
            "disable_web_page_preview": True,
        }
        if parse_mode:
            payload["parse_mode"] = parse_mode
        if reply_parameters is not None:
            payload["reply_parameters"] = reply_parameters
        if message_thread_id is not None:
            payload["message_thread_id"] = message_thread_id
        url = f"https://api.telegram.org/bot{token}/sendMessage"
        req = Request(
            url,
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urlopen(req, timeout=20) as r:
                result = json.loads(r.read())
                result["used_chat_id"] = payload["chat_id"]
                return result
        except HTTPError as e:
            raw = e.read().decode("utf-8", errors="replace")
            last_err = f"Telegram HTTP {e.code}: {raw}"
            tried.append({"candidate": payload["chat_id"], "error": last_err})
    raise RuntimeError(f"All chat id candidates failed: {json.dumps(tried, ensure_ascii=False)}")


class Handler(BaseHTTPRequestHandler):
    def _send_json(self, code: int, payload: dict) -> None:
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parsed = urlparse(self.path)
        if parsed.path == "/health":
            self._send_json(200, {"ok": True})
            return
        self._send_json(404, {"ok": False, "error": "not found"})

    def do_POST(self):
        parsed = urlparse(self.path)
        length = int(self.headers.get("Content-Length", "0") or "0")
        raw = self.rfile.read(length) if length > 0 else b""
        text = ""
        parse_mode = ""
        reply_to_message_id = None
        message_thread_id = None
        chat_id = None

        if raw:
            try:
                data = json.loads(raw.decode("utf-8"))
            except Exception:
                data = {}
            if isinstance(data, dict):
                text = str(data.get("text") or data.get("message") or "")
                parse_mode = str(data.get("parse_mode") or "")
                reply_to_message_id = data.get("reply_to_message_id")
                message_thread_id = data.get("message_thread_id")
                chat_id = data.get("chat_id")
        if not text and self.headers.get("Content-Type", "").startswith("application/x-www-form-urlencoded"):
            form = parse_qs(raw.decode("utf-8"))
            text = (form.get("text") or form.get("message") or [""])[0]
            parse_mode = (form.get("parse_mode") or [""])[0]
            reply_to_message_id = (form.get("reply_to_message_id") or [None])[0]
            message_thread_id = (form.get("message_thread_id") or [None])[0]
            chat_id = (form.get("chat_id") or [None])[0]

        if parsed.path != "/send":
            self._send_json(404, {"ok": False, "error": "not found"})
            return
        if not text:
            self._send_json(400, {"ok": False, "error": "missing text"})
            return

        try:
            result = telegram_send_auto(
                text,
                parse_mode=parse_mode,
                reply_to_message_id=int(reply_to_message_id) if reply_to_message_id not in (None, "", "None") else None,
                message_thread_id=int(message_thread_id) if message_thread_id not in (None, "", "None") else None,
                chat_id=str(chat_id) if chat_id not in (None, "", "None") else None,
            )
            self._send_json(200, {"ok": True, "result": result})
        except Exception as e:
            self._send_json(500, {"ok": False, "error": str(e)})

    def log_message(self, fmt, *args):
        return


def run_server() -> None:
    host = env("TELEGRAM_SERVER_HOST", "127.0.0.1")
    port = int(env("TELEGRAM_SERVER_PORT", "8787"))
    server = ThreadingHTTPServer((host, port), Handler)
    print(f"telegram relay listening on http://{host}:{port}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


def main():
    load_dotenv()
    ap = argparse.ArgumentParser(description="Telegram relay/server")
    ap.add_argument("--serve", action="store_true", help="Run local HTTP relay server")
    ap.add_argument("--send", default="", help="Send one message directly to Telegram")
    ap.add_argument("--parse-mode", default="", help="Telegram parse_mode for --send")
    ap.add_argument("--get-updates", action="store_true", help="Fetch raw Telegram updates")
    ap.add_argument("--resolve-chat-id", action="store_true", help="Extract chat ids from Telegram updates")
    ap.add_argument("--resolve-env-chat", action="store_true", help="Validate the chat id from .env via getChat")
    ap.add_argument("--poll-updates", action="store_true", help="Wait for the next Telegram update and print chat ids")
    args = ap.parse_args()

    if args.get_updates:
        print(json.dumps(telegram_get_updates(), ensure_ascii=False, indent=2))
        return

    if args.resolve_chat_id:
        print(json.dumps(resolve_chat_ids(telegram_get_updates()), ensure_ascii=False, indent=2))
        return

    if args.resolve_env_chat:
        print(json.dumps(resolve_env_chat(), ensure_ascii=False, indent=2))
        return

    if args.poll_updates:
        last = telegram_get_updates()
        offset = None
        results = last.get("result", []) or []
        if results:
            offset = max((item.get("update_id", 0) for item in results if isinstance(item, dict)), default=None)
            if offset is not None:
                offset += 1
        print("waiting for next telegram update...")
        while True:
            updates = telegram_poll_updates(offset=offset, timeout=30)
            results = updates.get("result", []) or []
            if not results:
                continue
            print(json.dumps(resolve_chat_ids(updates), ensure_ascii=False, indent=2))
            if results:
                offset = max((item.get("update_id", 0) for item in results if isinstance(item, dict)), default=None)
                if offset is not None:
                    offset += 1
            return

    if args.send:
        if env("TELEGRAM_NOTIFY_URL"):
            result = forward_to_notify_url(args.send)
        else:
            result = telegram_send_auto(args.send, parse_mode=args.parse_mode)
        print(json.dumps(result, ensure_ascii=False))
        return

    if args.serve:
        run_server()
        return

    ap.error("choose --serve, --send, --get-updates, --resolve-chat-id, --resolve-env-chat, or --poll-updates")


if __name__ == "__main__":
    main()
