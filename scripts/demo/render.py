"""Renders the fsearch vs Everything vs Windows Search demo video from the
measurements collect.ps1 takes.

  python render.py demo.json out.mp4 [explorer.json]   (needs Pillow and ffmpeg)

explorer.json (from bench-explorer-search.ps1) adds the File Explorer race.
"""

import json
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont

W, H, FPS = 1600, 900, 30
BG = (40, 40, 58)
CARD = (24, 24, 26)
TEXT = (236, 236, 236)
DIM = (128, 128, 134)
FAINT = (70, 70, 76)
CURSOR = (110, 156, 255)
ENGINES = [
    ("fsearch", "fsearch", (120, 220, 130)),
    ("everything", "Everything 1.4", (245, 177, 76)),
    ("windows", "Windows Search", (255, 107, 107)),  # substring (LIKE), as the others match
]

MONO = "C:/Windows/Fonts/CascadiaMono.ttf"


def font(size, bold=False):
    f = ImageFont.truetype(MONO, size)
    try:
        f.set_variation_by_name("Bold" if bold else "Regular")
    except Exception:
        pass
    return f


F = {k: font(s, b) for k, (s, b) in {
    "title": (34, True), "big": (26, True), "body": (20, False), "bold": (20, True), "small": (17, False), "tiny": (15, False),
}.items()}

data = json.load(open(sys.argv[1], encoding="utf-8-sig"))
if len(sys.argv) > 3:
    data["explorer"] = json.load(open(sys.argv[3], encoding="utf-8-sig"))
HOME = data["home"]
ENTRIES = data["entries"]


def card(d):
    x0, y0, x1, y1 = 60, 50, W - 60, H - 50
    d.rounded_rectangle((x0, y0, x1, y1), 22, fill=CARD)
    for i, c in enumerate([(255, 95, 87), (254, 188, 46), (40, 200, 64)]):
        d.ellipse((x0 + 22 + i * 20, y0 + 20, x0 + 34 + i * 20, y0 + 32), fill=c)
    return x0 + 60, y0 + 70


def fmt_ms(ms):
    if ms < 10:
        return f"{ms:.2f} ms"
    if ms < 1000:
        return f"{ms:.0f} ms"
    return f"{ms / 1000:.1f} s"


def short(path, width):
    """'name  ~\\parent' within `width` characters."""
    p = path.replace(HOME, "~", 1)
    cut = p.rfind("\\")
    name, parent = p[cut + 1:], p[:cut]
    if len(name) >= width:
        return name[: width - 1] + "\u2026", ""
    room = width - len(name) - 2
    if len(parent) > room:
        parent = parent[: max(0, room - 1)] + "\u2026" if room > 1 else ""
    return name, parent


def lerp(a, b, t):
    return a + (b - a) * max(0.0, min(1.0, t))


def ease(t):
    t = max(0.0, min(1.0, t))
    return 1 - (1 - t) ** 3


# Scenes: each is (frames, draw(d, frame)).

def title_scene():
    def draw(d, f):
        x, y = card(d)
        d.text((x, y + 160), "fsearch for Windows", font=F["title"], fill=TEXT)
        d.text((x, y + 215), f"whole disk \u00b7 {ENTRIES:,} files and folders \u00b7 Ryzen 5 7600X", font=F["body"], fill=DIM)
        t = ease((f - 20) / 20)
        if t > 0:
            for i, (_, label, color) in enumerate(ENGINES[1:]):
                d.text((x, y + 300 + i * 40), f"vs {label}", font=F["big"], fill=tuple(int(lerp(c * 0.2, c, t)) for c in color))
        d.text((x, y + 470), "every keystroke is a whole-disk search; same queries, top 50 with paths", font=F["small"], fill=DIM)
    return 90, draw


def typing_scene(item):
    q, steps = item["query"], item["steps"]
    per = 7  # frames per keystroke
    hold = 75
    n = len(steps)

    def draw(d, f):
        x, y = card(d)
        k = min(n, f // per + 1)  # keystrokes typed so far
        s = steps[k - 1]
        d.text((x, y - 6), "Every keystroke, three engines", font=F["bold"], fill=TEXT)
        d.text((x + 400, y - 4), f"whole disk \u00b7 {ENTRIES:,} files and folders", font=F["small"], fill=DIM)
        # The query line with a blinking cursor.
        d.text((x, y + 40), "\u276f", font=F["big"], fill=CURSOR)
        d.text((x + 30, y + 40), s["prefix"], font=F["big"], fill=TEXT)
        cw = d.textlength(s["prefix"], font=F["big"])
        if (f // 15) % 2 == 0 or f < n * per:
            d.rectangle((x + 32 + cw, y + 42, x + 44 + cw, y + 72), fill=CURSOR)
        colw = (W - 120 - 120) // 3
        for i, (key, label, color) in enumerate(ENGINES):
            cx = x + i * (colw + 30)
            r = s[key]
            d.text((cx, y + 110), label, font=F["bold"], fill=color)
            d.text((cx + colw - 150, y + 110), fmt_ms(r["ms"]).rjust(10), font=F["bold"], fill=color)
            d.line((cx, y + 142, cx + colw - 20, y + 142), fill=FAINT, width=1)
            count = r["n"]
            d.text((cx, y + 152), f"{count}{'+' if count >= 50 else ''} results" if count else "no results", font=F["tiny"], fill=DIM)
            width_chars = int((colw - 20) / d.textlength("m", font=F["small"]))
            for j, path in enumerate(r["top"][:7]):
                name, parent = short(path, width_chars)
                yy = y + 185 + j * 30
                d.text((cx, yy), name, font=F["small"], fill=TEXT)
                if parent:
                    d.text((cx + d.textlength(name + "  ", font=F["small"]), yy), parent, font=F["small"], fill=DIM)
            # Histogram of keystroke latencies so far, on one scale for all
            # three engines (so the difference shows).
            times = [steps[m][key]["ms"] for m in range(k)]
            # Scaled to typical keystrokes (an engine's first, one-letter
            # keystroke can be an outlier); taller bars are clipped and marked.
            meds = [sorted(st[e]["ms"] for st in steps)[n // 2] for e, _, _ in ENGINES]
            top = 2.5 * max(meds) or 1
            hx, hy, hh = cx, y + 420, 120
            bw = max(4, int((colw - 40) / max(n, 1)) - 4)
            for m, t in enumerate(times):
                bh = max(3, min(hh, int(hh * t / top)))
                bx = hx + m * (bw + 4)
                d.rectangle((bx, hy + hh - bh, bx + bw, hy + hh), fill=color)
                if t > top:
                    d.polygon([(bx, hy - 4), (bx + bw, hy - 4), (bx + bw / 2, hy - 12)], fill=color)
            med = sorted(times)[len(times) // 2]
            d.text((cx, hy + hh + 12), f"median {fmt_ms(med)}", font=F["small"], fill=color)
    return n * per + hold, draw


def bars_scene():
    bench = data["bench"]
    rows = ENGINES + [("windows_prefix", "Windows Search*", (190, 110, 110))]
    med = {}
    for key, _, _ in rows:
        ts = sorted(b[key]["ms"] for b in bench)
        med[key] = ts[len(ts) // 2]
    chrome = next((b for b in bench if b["query"] == "chrome.exe"), bench[0])

    def draw(d, f):
        x, y = card(d)
        d.text((x, y - 6), "Find a file by name", font=F["bold"], fill=TEXT)
        d.text((x + 260, y - 4), f"median of {len(bench)} queries \u00b7 top 50 with paths \u00b7 warm", font=F["small"], fill=DIM)
        t = ease(f / 30)
        full = W - 120 - 700  # room for the labels after the longest bar
        top = max(med.values())
        for i, (key, label, color) in enumerate(rows):
            yy = y + 60 + i * 62
            d.text((x, yy + 6), label, font=F["body"], fill=TEXT)
            bw = max(6, int(full * med[key] / top * t))
            d.rectangle((x + 200, yy, x + 200 + bw, yy + 36), fill=color)
            label_ms = fmt_ms(med[key] * t if t < 1 else med[key])
            d.text((x + 214 + bw, yy + 6), label_ms, font=F["bold"], fill=color)
            if key != "fsearch" and t >= 1:
                d.text((x + 214 + bw + d.textlength(label_ms + "  ", font=F["bold"]), yy + 6), f"{med[key] / med['fsearch']:.0f}\u00d7 slower", font=F["body"], fill=DIM)
        if t >= 1:
            d.text((x, y + 310), "* word-prefix index lookup: faster, but only matches the start of words (no “ome.ex”)", font=F["small"], fill=DIM)
        # Coverage: what each finds for chrome.exe.
        t2 = ease((f - 45) / 25)
        if t2 > 0:
            yy = y + 370
            d.text((x, yy), f"Results for \u201c{chrome['query']}\u201d", font=F["bold"], fill=TEXT)
            for i, (key, label, color) in enumerate(ENGINES):
                n = chrome[key]["n"]
                d.text((x, yy + 45 + i * 34), label, font=F["body"], fill=TEXT)
                d.text((x + 260, yy + 45 + i * 34), f"{n}{'+' if n >= 50 else ''}", font=F["bold"], fill=color)
            d.text((x, yy + 160), "Windows Search only indexes your user folders, so it misses Program Files.", font=F["small"], fill=DIM)
    return 165, draw


def fmt_long(sec):
    m, s = divmod(sec, 60)
    return f"{int(m)}m {s:04.1f}s" if m else f"{s:.1f} s"


def explorer_scene(ex):
    """Find a file on the whole drive: the engines against File Explorer's
    own search (which crawls whatever isn't indexed)."""
    q = ex["query"]
    b = next((r for r in data["bench"] if r["query"] == q), data["bench"][0])
    rows = [("fsearch", "fsearch", ENGINES[0][2], b["fsearch"]["ms"] / 1000), ("everything", "Everything 1.4", ENGINES[1][2], b["everything"]["ms"] / 1000),
            ("explorer", "File Explorer", (110, 156, 255), ex["done_s"])]
    lapse = 3.0  # seconds of video for File Explorer's whole search

    def draw(d, f):
        x, y = card(d)
        d.text((x, y - 6), f"Find “{q}” on all of {ex['location'].rstrip(chr(92))}", font=F["bold"], fill=TEXT)
        d.text((x + 470, y - 4), "File Explorer’s own search, start to finish", font=F["small"], fill=DIM)
        now = f / FPS  # video seconds into the scene
        full = W - 120 - 900
        total = rows[-1][3]
        for i, (key, label, color, secs) in enumerate(rows):
            yy = y + 70 + i * 80
            d.text((x, yy + 8), label, font=F["body"], fill=TEXT)
            # Engines finish in the first frame; Explorer runs in time-lapse.
            shown = secs if key != "explorer" else min(secs, total * now / lapse)
            bw = max(6, int(full * shown / total))
            d.rectangle((x + 220, yy, x + 220 + bw, yy + 40), fill=color)
            done = shown >= secs
            if key == "explorer" and done and ex.get("timed_out"):
                label_t = f"{round(secs / 60)}m+"
            else:
                label_t = fmt_ms(shown * 1000) if secs < 1 else fmt_long(shown)
            d.text((x + 234 + bw, yy + 8), label_t, font=F["bold"], fill=color)
            if key == "explorer" and not done:
                d.text((x + 234 + bw + d.textlength(label_t + "  ", font=F["bold"]), yy + 10), f"time-lapse {total / lapse:.0f}×", font=F["small"], fill=DIM)
            if key == "explorer" and done:
                note = f"still searching · {ex['items']} found so far" if ex.get("timed_out") else f"{ex['items']} found"
                if ex.get("first_result_s"):
                    note += f" · first result at {fmt_long(ex['first_result_s'])}"
                d.text((x + 220, yy + 50), note, font=F["small"], fill=DIM)
        if now > lapse + 0.4:
            t = ease((now - lapse - 0.4) / 0.6)
            ratio = total / rows[0][3]
            more = "over " if ex.get("timed_out") else ""
            d.text((x, y + 340), f"{more}{ratio:,.0f}× faster than File Explorer", font=F["title"], fill=tuple(int(lerp(CARD[i], ENGINES[0][2][i], t)) for i in range(3)))
            d.text((x, y + 400), f"fsearch found {b['fsearch']['n']}{'+' if b['fsearch']['n'] >= 50 else ''} in {fmt_ms(rows[0][3] * 1000)}; Explorer crawls every folder Windows Search doesn’t index.", font=F["small"], fill=DIM)
    return int((lapse + 3.2) * FPS), draw


def outro_scene():
    def draw(d, f):
        x, y = card(d)
        d.text((x, y + 180), "Finder for Windows", font=F["title"], fill=TEXT)
        d.text((x, y + 235), "github.com/pranavkarthik10/Finder", font=F["big"], fill=ENGINES[0][2])
        d.text((x, y + 300), "search engine: fsearch by Noah Dunnagan, ported to Windows", font=F["body"], fill=DIM)
        d.text((x, y + 335), "all numbers measured on this PC", font=F["body"], fill=DIM)
    return 90, draw


scenes = [title_scene()] + [typing_scene(t) for t in data["typed"]] + [bars_scene()]
if data.get("explorer"):
    scenes.append(explorer_scene(data["explorer"]))
scenes.append(outro_scene())
out = sys.argv[2]
ff = subprocess.Popen(
    ["ffmpeg", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-",
     "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-preset", "slow", "-movflags", "+faststart", out],
    stdin=subprocess.PIPE,
)
total = 0
for frames, draw in scenes:
    for f in range(frames):
        img = Image.new("RGB", (W, H), BG)
        draw(ImageDraw.Draw(img), f)
        ff.stdin.write(img.tobytes())
        total += 1
ff.stdin.close()
ff.wait()
print(f"{total} frames, {total / FPS:.1f}s -> {out}")
