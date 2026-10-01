#!/usr/bin/env python3
"""Take the README screenshots (run it through scripts/screenshots.sh, which
builds Gibbon and the tools first).

    scripts/screenshots/take.py <work folder> [view...]

For each view, in dark and in light: start Gibbon on the demo repository as
a GIBBON_BACKGROUND pop-up with a crafted session, capture its window, frame
the capture and write docs/screenshots/<view>-<dark|light>.png as a
256-color PNG. Without views, it takes all of them."""
import json
import os
import signal
import subprocess
import sys
import time

import imagequant
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
WORK = os.path.abspath(sys.argv[1])
HOME = f"{WORK}/home"
REPO = f"{HOME}/Developer/gibbon"
APP = f"{ROOT}/target/release/gibbon"
OUT = f"{ROOT}/docs/screenshots"
# The window in points. The captures are twice as large.
WINDOW = {"x": 120, "y": 80, "w": 1200, "h": 760}
# The width of the framed PNG: sharp at the width of a GitHub README on a
# Retina display.
WIDTH = 1800
ENV = dict(os.environ, HOME=HOME, GIBBON_BACKGROUND="1", GIT_CONFIG_GLOBAL="/dev/null")


def git(*args):
    return subprocess.run(
        ["git", "-C", REPO, *args], env=ENV, capture_output=True, text=True, check=True
    ).stdout.strip()


def sha(branch, grep):
    return git("log", "-1", "--format=%H", f"--grep={grep}", branch)


subprocess.run([sys.executable, f"{ROOT}/scripts/screenshots/demo.py", WORK], check=True)

# view: (seconds to wait, repository state in session.json, environment)
VIEWS = {
    "history": (6, {"view": "history", "target": "all",
                    "commit": sha("claude/badges", "badge the commits")}, {}),
    "changes": (6, {}, {"GIBBON_VIEW": "changes", "GIBBON_FILE": "src/theme.rs"}),
    "pick": (6, {"view": "history", "target": "refs/heads/codex/new-worktree",
                 "commit": sha("codex/new-worktree", "New Worktree dialog")}, {}),
    "review": (7, {"view": "review", "file": 1, "review": {
        "target": "refs/heads/claude/badges", "base": "refs/heads/main",
        "worktree": f"{HOME}/Developer/gibbon-worktrees/claude-badges"}}, {}),
    "activity": (6, {}, {"GIBBON_VIEW": "activity"}),
    "cleanup": (8, {}, {"GIBBON_DIALOG": "cleanup"}),
    "settings": (6, {}, {"GIBBON_DIALOG": "settings"}),
}


def capture(out, mode, wait, state, env):
    data = f"{HOME}/Library/Application Support/gibbon"
    os.makedirs(data, exist_ok=True)
    with open(f"{data}/settings.json", "w") as f:
        json.dump({"appearance": mode, "auto_fetch": 0}, f)
    with open(f"{data}/session.json", "w") as f:
        json.dump({"window": WINDOW, "tabs": [REPO], "active": 0, "repos": {REPO: state}}, f)
    if os.path.exists(f"{data}/recent.json"):
        os.remove(f"{data}/recent.json")
    app = subprocess.Popen(
        [APP, REPO], env={**ENV, **env}, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    try:
        time.sleep(wait)
        win = subprocess.run(
            [f"{WORK}/winpid", str(app.pid)], capture_output=True, text=True
        ).stdout.strip()
        if not win:
            raise SystemExit("Gibbon opened no window")
        # A sleeping display gives no image: keep it awake.
        subprocess.run(["screencapture", "-x", "-o", f"-l{win}", out], check=True)
    finally:
        app.send_signal(signal.SIGTERM)
        app.wait()


def buttons_gray(path):
    """macOS draws the window buttons gray now and then. The close button
    is in the top left corner: look for its red."""
    im = Image.open(path).convert("RGB")
    red = max(
        (im.getpixel((x, y)) for x in range(10, 60) for y in range(10, 60)),
        key=lambda p: p[0] - p[2],
    )
    return red[0] - red[2] < 80


os.makedirs(f"{WORK}/raw", exist_ok=True)
os.makedirs(OUT, exist_ok=True)
for view in sys.argv[2:] or VIEWS:
    wait, state, env = VIEWS[view]
    for mode in ("dark", "light"):
        name = f"{view}-{mode}"
        raw, framed = f"{WORK}/raw/{name}.png", f"{WORK}/raw/{name}-framed.png"
        for _ in range(5):
            capture(raw, mode, wait, state, env)
            if not buttons_gray(raw):
                break
        else:
            print(f"{name}: the window buttons stayed gray")
        subprocess.run(
            [f"{WORK}/frame", raw, framed, str(WIDTH), "1" if mode == "dark" else "0"],
            check=True, stdout=subprocess.DEVNULL,
        )
        png = imagequant.quantize_pil_image(
            Image.open(framed).convert("RGBA"), dithering_level=1.0, max_colors=256
        )
        png.save(f"{OUT}/{name}.png", optimize=True)
        print(f"docs/screenshots/{name}.png", os.path.getsize(f"{OUT}/{name}.png") // 1024, "KB")
