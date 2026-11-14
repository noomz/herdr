#!/usr/bin/env python3
"""Calibration probe for neru hints (SPEC-neru-hints.md T1).

Run it directly in a host terminal window (kitty or Ghostty), not inside herdr,
with the neru daemon running. It:

1. reads the tty's ioctl grid and pixel size, neru's focused window and the
   pointer position;
2. moves the pointer to two points inside the window with `neru action
   move_mouse`, once with SGR cell reports and once with SGR pixel reports
   (?1016h), and records what the terminal reports;
3. derives the grid origin and the pixel scale from those reports (the
   "move and observe" method) and the offset from neru's window frame to the
   grid (the "window + fixed offset" method);
4. clicks the center of a few known cells with `left_click --bare` and records
   which cell the terminal reports, for each method;
5. dumps `kitty @ ls` geometry for this window when kitty remote control is on;
6. moves the pointer back and restores the terminal modes.

A click is sent only after a motion report shows the pointer is over this
window, so a click never lands in another app.

Usage: probe.py [--label NAME] [--out results.json] [--timeout-ms 400] [--kitty-to unix:/path]
"""

import argparse
import fcntl
import json
import os
import re
import select
import struct
import subprocess
import sys
import termios
import time
import tty

SGR = re.compile(rb"\x1b\[<(\d+);(\d+);(\d+)([Mm])")
MOTION_NO_BUTTON = 35
LEFT_PRESS = 0


def neru(*args):
    out = subprocess.run(["neru", *args], capture_output=True, text=True, timeout=10)
    if out.returncode != 0:
        raise RuntimeError(f"neru {' '.join(args)}: {out.stderr.strip() or out.stdout.strip()}")
    return out.stdout


def neru_json(*args):
    return json.loads(neru(*args, "--json"))


def find_rect(obj):
    """Return the first dict holding x, y, width, height, searching depth-first."""
    if isinstance(obj, dict):
        if all(k in obj for k in ("x", "y", "width", "height")):
            return obj
        for v in obj.values():
            r = find_rect(v)
            if r:
                return r
    if isinstance(obj, list):
        for v in obj:
            r = find_rect(v)
            if r:
                return r
    return None


def find_point(obj):
    if isinstance(obj, dict):
        if "x" in obj and "y" in obj:
            return obj
        for v in obj.values():
            r = find_point(v)
            if r:
                return r
    return None


def host_app_pid():
    """PID of the nearest ancestor that is a macOS app bundle executable."""
    pid = os.getppid()
    while pid > 1:
        out = subprocess.run(["ps", "-o", "ppid=,comm=", "-p", str(pid)], capture_output=True, text=True)
        ppid, _, comm = out.stdout.strip().partition(" ")
        if ".app/Contents/MacOS/" in comm:
            return pid
        pid = int(ppid or 0)
    return None


def winsize(fd):
    rows, cols, xpx, ypx = struct.unpack("HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\0" * 8))
    return {"cols": cols, "rows": rows, "xpixel": xpx, "ypixel": ypx}


class Tty:
    def __init__(self):
        self.fd = os.open("/dev/tty", os.O_RDWR)
        self.saved = termios.tcgetattr(self.fd)
        self.buf = b""
        self.raw = b""

    def __enter__(self):
        tty.setraw(self.fd)
        self.write("\x1b[?1003h\x1b[?1006h\x1b[?1004h")
        return self

    def __exit__(self, *exc):
        self.write("\x1b[?1004l\x1b[?1016l\x1b[?1006l\x1b[?1003l")
        termios.tcsetattr(self.fd, termios.TCSADRAIN, self.saved)
        os.close(self.fd)

    def write(self, s):
        os.write(self.fd, s.encode())

    def pixel_mode(self, on):
        # ?1016l resets the report encoding to legacy X10 in kitty, so re-assert SGR.
        self.write("\x1b[?1016h" if on else "\x1b[?1016l\x1b[?1006h")
        self.drain(0.1)

    def wait_focus(self, secs):
        """Wait for a focus-in report (CSI I) from ?1004h."""
        end = time.monotonic() + secs
        while time.monotonic() < end:
            r, _, _ = select.select([self.fd], [], [], max(0, end - time.monotonic()))
            if r:
                self.raw += os.read(self.fd, 4096)
                if b"\x1b[I" in self.raw:
                    return True
        return False

    def drain(self, secs):
        end = time.monotonic() + secs
        while time.monotonic() < end:
            r, _, _ = select.select([self.fd], [], [], max(0, end - time.monotonic()))
            if r:
                self.raw += os.read(self.fd, 4096)
        self.buf = b""

    def reports(self, secs):
        """Collect SGR reports for `secs` seconds."""
        end = time.monotonic() + secs
        out = []
        while time.monotonic() < end:
            r, _, _ = select.select([self.fd], [], [], max(0, end - time.monotonic()))
            if r:
                data = os.read(self.fd, 4096)
                self.raw += data
                self.buf += data
                last = 0
                for m in SGR.finditer(self.buf):
                    b, x, y, kind = m.groups()
                    out.append({"b": int(b), "x": int(x), "y": int(y), "press": kind == b"M"})
                    last = m.end()
                rest = self.buf[last:]
                cut = rest.rfind(b"\x1b")
                self.buf = rest[cut:] if cut >= 0 else b""
        return out


INTERFERENCE = []


def cursor_at(pt):
    c = find_point(neru_json("query", "cursor"))
    return c and abs(c["x"] - pt[0]) <= 1 and abs(c["y"] - pt[1]) <= 1


def move_and_observe(t, pt, timeout, tries=3):
    """Warp to `pt` and return the first motion report the warp produced.

    A warp yields one report. Later reports come from someone moving the real
    pointer, so they are ignored, and the move is retried if the pointer has
    left `pt` by the time it is checked.
    """
    for _ in range(tries):
        t.drain(0.05)
        neru("action", "move_mouse", "--x", str(pt[0]), "--y", str(pt[1]))
        motions = [r for r in t.reports(timeout) if r["b"] == MOTION_NO_BUTTON]
        if not motions:
            return None
        if cursor_at(pt):
            return motions[0]
        INTERFERENCE.append({"point": pt, "reports": len(motions)})
    return None


def click_at(t, pt, timeout):
    """Move, confirm the pointer is over this window, click, return the press report."""
    if move_and_observe(t, pt, timeout) is None:
        return {"skipped": "no clean motion report: pointer not over the probe window"}
    neru("action", "left_click", "--bare")
    presses = [r for r in t.reports(timeout) if r["b"] == LEFT_PRESS and r["press"]]
    return presses[0] if presses else {"skipped": "no press report"}


def kitty_geometry(to):
    to = to or os.environ.get("KITTY_LISTEN_ON")
    if not to or not os.environ.get("KITTY_WINDOW_ID"):
        return None
    out = subprocess.run(["kitty", "@", "--to", to, "ls"], capture_output=True, text=True, timeout=10)
    if out.returncode != 0:
        return {"error": out.stderr.strip()}
    wid = int(os.environ["KITTY_WINDOW_ID"])
    drop = {"env", "cmdline", "foreground_processes", "user_vars", "last_reported_cmdline"}
    for osw in json.loads(out.stdout):
        for tab in osw.get("tabs", []):
            for w in tab.get("windows", []):
                if w.get("id") == wid:
                    return {
                        "os_window": {k: v for k, v in osw.items() if k != "tabs"},
                        "window": {k: v for k, v in w.items() if k not in drop},
                    }
    return {"error": f"window {wid} not found"}


def edge_search(t, pt, axis, cell_pt, timeout):
    """Step one point at a time along `axis` until the reported cell changes.

    Cell-mode motion is reported only when the cell changes, so a step with no
    report means "same cell". Returns (screen coordinate of the first point in
    the next cell, that cell's 1-based index), or None. This is the ground
    truth for the grid origin, good to one point.
    """
    start = move_and_observe(t, pt, timeout)
    if start is None:
        return None
    key = "x" if axis == 0 else "y"
    for step in range(1, int(cell_pt) + 3):
        p = list(pt)
        p[axis] += step
        t.drain(0.05)
        neru("action", "move_mouse", "--x", str(p[0]), "--y", str(p[1]))
        motions = [r for r in t.reports(timeout) if r["b"] == MOTION_NO_BUTTON]
        if motions and motions[0][key] != start[key]:
            if not cursor_at(p):
                INTERFERENCE.append({"edge_step": p})
                return None
            return p[axis], motions[0][key]
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", default="run")
    ap.add_argument("--out")
    ap.add_argument("--timeout-ms", type=int, default=400)
    ap.add_argument("--kitty-to", help="kitty remote control socket, e.g. unix:/path")
    ap.add_argument("--search", action="store_true",
                    help="find a point inside this terminal by trying a 3x3 grid (kitty splits)")
    ap.add_argument("--focus-wait", type=float, default=5, help="seconds to wait for focus-in")
    args = ap.parse_args()
    timeout = args.timeout_ms / 1000

    res = {"label": args.label, "term": os.environ.get("TERM")}
    with Tty() as t:
        ws = winsize(t.fd)
        res["winsize"] = ws
        if args.out:
            pid = str(host_app_pid() or "")
            with open(args.out + ".ready.tmp", "w") as f:
                f.write(pid)
            os.rename(args.out + ".ready.tmp", args.out + ".ready")
        # Advisory: no focus-in also happens when the window was focused from the
        # start. The motion-report guard keeps clicks inside this window anyway.
        res["focus_event"] = t.wait_focus(args.focus_wait)
        measure(t, ws, res, args, timeout)
        res["raw"] = repr(t.raw[-3000:])
        res["interference"] = INTERFERENCE

    res["cursor_after"] = neru_json("query", "cursor")
    text = json.dumps(res, indent=2)
    if args.out:
        with open(args.out, "w") as f:
            f.write(text + "\n")
    print(text)


def measure(t, ws, res, args, timeout):
    res["window"] = neru_json("query", "window")
    res["displays"] = neru_json("query", "displays")
    res["cursor_before"] = neru_json("query", "cursor")
    res["kitty"] = kitty_geometry(args.kitty_to)
    win = find_rect(res["window"])
    pre = find_point(res["cursor_before"])
    wx, wy, ww, wh = (win[k] for k in ("x", "y", "width", "height"))
    p1 = (int(wx + ww * 0.35), int(wy + wh * 0.40))
    p2 = (int(wx + ww * 0.65), int(wy + wh * 0.60))
    started = time.monotonic()

    try:
        if args.search:
            # The OS window may hold several terminal windows (kitty splits). Only
            # the one running this probe reports motion, so try points until one does.
            tries = 0
            for fy in (0.2, 0.5, 0.8):
                for fx in (0.2, 0.5, 0.8):
                    tries += 1
                    pt = (int(wx + ww * fx), int(wy + wh * fy))
                    if move_and_observe(t, pt, timeout):
                        p1, p2 = pt, (pt[0] + 40, pt[1] + 40)
                        break
                else:
                    continue
                break
            else:
                res["error"] = f"no point in the window frame reported motion ({tries} tries)"
                return
            res["search_tries"] = tries
        res["points"] = [p1, p2]
        obs = {}
        for mode in ("cell", "pixel"):
            t.pixel_mode(mode == "pixel")
            obs[mode] = [move_and_observe(t, p, timeout) for p in (p1, p2)]
        res["observed"] = obs
        px = obs["pixel"]
        if not all(px):
            res["error"] = "no motion report at a probe point"
            return

        # Move and observe, pixel variant: pixel scale and pixel-space origin.
        sx = (px[1]["x"] - px[0]["x"]) / (p2[0] - p1[0])
        sy = (px[1]["y"] - px[0]["y"]) / (p2[1] - p1[1])
        # Integer cell pitch, as herdr's ioctl_cell_size computes it. The ioctl
        # extent can include padding (Ghostty), so a float division drifts.
        cw = (ws["xpixel"] // ws["cols"]) / sx
        ch = (ws["ypixel"] // ws["rows"]) / sy
        origins = {"pixel": (p1[0] - px[0]["x"] / sx, p1[1] - px[0]["y"] / sy)}
        calib = {"scale": [sx, sy], "cell_pt": [cw, ch]}

        # Ground truth: find a cell boundary in cell mode, one point at a time.
        t.pixel_mode(False)
        ex = edge_search(t, p1, 0, cw, timeout)
        ey = edge_search(t, p1, 1, ch, timeout)
        calib["edges"] = [ex, ey]
        if ex and ey:
            origins["edge"] = (ex[0] - (ex[1] - 1) * cw, ey[0] - (ey[1] - 1) * ch)
        calib["origins"] = origins
        calib["window_offset"] = {k: (o[0] - wx, o[1] - wy) for k, o in origins.items()}
        res["calibration"] = calib

        clicks = []
        targets = [(2, 1), (ws["cols"] // 2, ws["rows"] // 2), (ws["cols"] - 1, ws["rows"])]
        for method, (ox, oy) in origins.items():
            for col, row in targets:  # 1-based, as SGR reports them
                pt = (round(ox + (col - 0.5) * cw), round(oy + (row - 0.5) * ch))
                inside = wx <= pt[0] < wx + ww and wy <= pt[1] < wy + wh
                rep = click_at(t, pt, timeout) if inside else {"skipped": "outside window frame"}
                clicks.append({"method": method, "cell": [col, row], "point": pt, "report": rep})
        res["clicks"] = clicks
    finally:
        res["elapsed_s"] = round(time.monotonic() - started, 2)
        if pre:
            neru("action", "move_mouse", "--x", str(int(pre["x"])), "--y", str(int(pre["y"])))
        t.drain(0.2)


if __name__ == "__main__":
    main()
