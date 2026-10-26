#!/usr/bin/env python3
"""Render docs/demo.gif: a scripted model of the scrolling strip, not a screen recording.

Windows follow the engine's behaviour (new columns open right of the focused one, the strip
scrolls just far enough to show the focused column) and move with a spring, with the pressed
shortcut and the minimap shown as overlays. Needs Pillow and ffmpeg; run from anywhere.
"""

import math
import os
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H, S = 800, 500, 2  # output size; drawn at S times that and scaled down for antialiasing
FPS = 25
MENU, DOCK, GAP = 26, 58, 12
TOP, BOTTOM = MENU + GAP, H - DOCK - GAP
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs", "demo.gif")


def font(names, size):
    for name in names:
        for d in ("/usr/share/fonts/opentype/inter", "/usr/share/fonts/truetype/dejavu", "/Library/Fonts"):
            try:
                return ImageFont.truetype(os.path.join(d, name), size * S)
            except OSError:
                pass
    return ImageFont.load_default(size * S)


UI = font(["Inter-Medium.otf", "DejaVuSans.ttf"], 11)
UI_BOLD = font(["Inter-SemiBold.otf", "DejaVuSans-Bold.ttf"], 11)
KEY = font(["DejaVuSans-Bold.ttf"], 17)
CAPTION = font(["Inter-SemiBold.otf", "DejaVuSans-Bold.ttf"], 13)
ICON = font(["Inter-Bold.otf", "DejaVuSans-Bold.ttf"], 12)

# app name -> (accent colour, content style)
APPS = {
    "Safari": ((52, 120, 246), "web"),
    "Terminal": ((40, 40, 46), "term"),
    "Notes": ((246, 196, 60), "notes"),
    "Code": ((0, 122, 204), "code"),
    "Mail": ((30, 144, 255), "mail"),
    "Music": ((250, 60, 90), "music"),
    "Finder": ((60, 170, 240), "notes"),
}


class Spring:
    """Critically damped-ish spring on a vector, like the app's slide animation."""

    def __init__(self, v):
        self.x, self.v = list(v), [0.0] * len(v)

    def step(self, target, dt=1 / FPS, k=210.0, c=25.0):
        for i, t in enumerate(target):
            a = k * (t - self.x[i]) - c * self.v[i]
            self.v[i] += a * dt
            self.x[i] += self.v[i] * dt


class Desk:
    def __init__(self):
        # workspace -> list of columns; a column is [width fraction, [app, ...]]
        self.ws = {
            1: [[0.5, ["Safari"]], [1 / 3, ["Terminal"]], [1 / 3, ["Notes"]]],
            2: [[0.5, ["Mail"]], [0.5, ["Music"]]],
        }
        self.cur, self.focus, self.row = 1, {1: 1, 2: 0}, {1: 0, 2: 0}
        self.scroll = {1: 0.0, 2: 0.0}
        self.springs, self.alpha = {}, {}
        self.key, self.key_t, self.caption = None, -99.0, ""
        self.map_t = -99.0
        self.t = 0.0
        self.layout(snap=True)

    def cols(self):
        return self.ws[self.cur]

    def targets(self, n):
        """Window frames for workspace n, keeping its focused column on screen."""
        usable = W - 2 * GAP
        cols, xs, x = self.ws[n], [], GAP
        for w, _ in cols:
            xs.append(x)
            x += w * usable + GAP
        f = self.focus[n]
        left, right = xs[f], xs[f] + cols[f][0] * usable
        s = self.scroll[n]
        if left - s < GAP:
            s = left - GAP
        elif right - s > W - GAP:
            s = right - W + GAP
        self.scroll[n] = s
        out = {}
        for (w, apps), cx in zip(cols, xs):
            h = (BOTTOM - TOP - GAP * (len(apps) - 1)) / len(apps)
            for r, app in enumerate(apps):
                y = TOP + r * (h + GAP) + (n - self.cur) * H
                out[app] = [cx - s, y, w * usable, h]
        return out

    def layout(self, snap=False):
        self.goal = {}
        for n in self.ws:
            self.goal.update(self.targets(n))
        for app, r in self.goal.items():
            if app not in self.springs or snap:
                self.springs[app] = Spring(r)
                self.alpha[app] = 1.0

    def press(self, keys, caption):
        self.key, self.key_t, self.caption = keys, self.t, caption

    def nav(self):
        self.map_t = self.t

    # commands, mirroring Engine::command
    def focus_col(self, d):
        f = self.focus[self.cur] + d
        self.focus[self.cur] = max(0, min(len(self.cols()) - 1, f))
        self.row[self.cur] = 0
        self.nav()

    def move_col(self, d):
        cols, f = self.cols(), self.focus[self.cur]
        g = f + d
        if 0 <= g < len(cols):
            cols[f], cols[g] = cols[g], cols[f]
            self.focus[self.cur] = g
        self.nav()

    def cycle_width(self):
        presets = [1 / 3, 0.5, 2 / 3]
        col = self.cols()[self.focus[self.cur]]
        nxt = [p for p in presets if p > col[0] + 0.01]
        col[0] = nxt[0] if nxt else presets[0]
        self.nav()

    def open_window(self, app):
        self.cols().insert(self.focus[self.cur] + 1, [0.5, [app]])
        self.focus[self.cur] += 1
        self.layout()
        r = self.goal[app]
        self.springs[app] = Spring([r[0] + r[2] * 0.08, r[1] + r[3] * 0.08, r[2] * 0.84, r[3] * 0.84])
        self.alpha[app] = 0.0
        self.nav()

    def consume(self):
        cols, f = self.cols(), self.focus[self.cur]
        if f + 1 < len(cols):
            cols[f][1] += cols.pop(f + 1)[1]
            self.row[self.cur] = len(cols[f][1]) - 1
        self.nav()

    def switch(self, n):
        self.cur = n

    def focused_app(self):
        col = self.cols()[self.focus[self.cur]]
        return col[1][min(self.row[self.cur], len(col[1]) - 1)]

    def tick(self):
        self.layout()
        for app, sp in self.springs.items():
            sp.step(self.goal[app])
            self.alpha[app] = min(1.0, self.alpha[app] + 0.12)
        self.t += 1 / FPS


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def wallpaper():
    small = Image.new("RGB", (64, 40))
    px = small.load()
    for y in range(40):
        for x in range(64):
            u, v = x / 63, y / 39
            top = lerp((88, 70, 190), (40, 120, 200), u)
            bot = lerp((220, 90, 120), (250, 160, 90), u)
            px[x, y] = lerp(top, bot, v * 0.9 + 0.1 * math.sin(u * 3))
    return small.resize((W * S, H * S), Image.BICUBIC).filter(ImageFilter.GaussianBlur(20))


def content(d, style, x, y, w, h, accent):
    """Fake window contents, enough to tell apps apart at a glance."""
    s = S
    if style == "term":
        d.rectangle([x, y, x + w, y + h], fill=(28, 28, 32))
        lines = [("$ cargo test", (120, 220, 140)), ("   Compiling paperwm-engine", (200, 200, 200)),
                 ("    Finished test", (200, 200, 200)), ("running 214 tests", (200, 200, 200)),
                 ("test result: ok. 214 passed", (120, 220, 140)), ("$ git status", (120, 220, 140)),
                 ("On branch main", (200, 200, 200)), ("$ _", (120, 220, 140))]
        for i, (txt, col) in enumerate(lines):
            if 14 * s + i * 16 * s < h:
                d.text((x + 10 * s, y + 8 * s + i * 16 * s), txt, font=UI, fill=col)
        return
    d.rectangle([x, y, x + w, y + h], fill=(250, 250, 252))
    if style == "web":
        d.rounded_rectangle([x + w * 0.2, y + 8 * s, x + w * 0.8, y + 24 * s], 6 * s, fill=(232, 232, 236))
        d.rounded_rectangle([x + 14 * s, y + 38 * s, x + w - 14 * s, y + 38 * s + h * 0.35], 8 * s,
                            fill=lerp(accent, (255, 255, 255), 0.55))
        for i in range(6):
            yy = y + 50 * s + h * 0.35 + i * 14 * s
            if yy < y + h - 10 * s:
                d.rounded_rectangle([x + 14 * s, yy, x + w * (0.85 - 0.1 * (i % 3)), yy + 6 * s], 3 * s,
                                    fill=(215, 215, 222))
    elif style == "code":
        d.rectangle([x, y, x + w, y + h], fill=(30, 32, 40))
        palette = [(198, 120, 221), (97, 175, 239), (152, 195, 121), (229, 192, 123), (171, 178, 191)]
        for i in range(int(h / (14 * s)) - 1):
            indent = [0, 1, 2, 2, 1, 2, 3, 2, 1, 0][i % 10] * 14 * s
            seg_x = x + 12 * s + indent
            for j in range(1 + (i * 7) % 3):
                ln = (24 + (i * 13 + j * 29) % 60) * s
                d.rounded_rectangle([seg_x, y + 10 * s + i * 14 * s, seg_x + ln, y + 16 * s + i * 14 * s], 3 * s,
                                    fill=palette[(i + j) % 5])
                seg_x += ln + 6 * s
    elif style == "music":
        art = min(w * 0.5, h * 0.5)
        d.rounded_rectangle([x + (w - art) / 2, y + 20 * s, x + (w + art) / 2, y + 20 * s + art], 10 * s,
                            fill=accent)
        d.rounded_rectangle([x + w * 0.2, y + 40 * s + art, x + w * 0.8, y + 46 * s + art], 3 * s,
                            fill=(215, 215, 222))
    else:  # notes, mail: a sidebar and lines
        side = min(w * 0.32, 90 * s)
        d.rectangle([x, y, x + side, y + h], fill=(240, 240, 244))
        for i in range(int(h / (34 * s))):
            yy = y + 10 * s + i * 34 * s
            d.rounded_rectangle([x + 8 * s, yy, x + side - 8 * s, yy + 24 * s], 5 * s,
                                fill=lerp(accent, (255, 255, 255), 0.4) if i == 0 else (228, 228, 234))
        for i in range(int(h / (16 * s)) - 1):
            yy = y + 14 * s + i * 16 * s
            d.rounded_rectangle([x + side + 12 * s, yy, x + w - (14 + (i * 23) % 50) * s, yy + 6 * s], 3 * s,
                                fill=(205, 205, 212))


def window(app, rect, focused, alpha):
    x, y, w, h = [v * S for v in rect]
    accent, style = APPS[app]
    pad = 24 * S
    layer = Image.new("RGBA", (int(w) + 2 * pad, int(h) + 2 * pad), (0, 0, 0, 0))
    shadow = Image.new("RGBA", layer.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle([pad, pad + 6 * S, pad + w, pad + h + 6 * S], 12 * S,
                                             fill=(0, 0, 0, 110 if focused else 60))
    layer = Image.alpha_composite(layer, shadow.filter(ImageFilter.GaussianBlur(10 * S)))
    body = Image.new("RGBA", layer.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(body)
    bar = 24 * S
    content(d, style, pad, pad + bar, w, h - bar, accent)
    dark = style in ("term", "code")
    d.rectangle([pad, pad, pad + w, pad + bar], fill=(52, 52, 58) if dark else (236, 236, 240))
    for i, c in enumerate([(255, 95, 87), (254, 188, 46), (40, 200, 64)]):
        cx = pad + (14 + i * 18) * S
        d.ellipse([cx - 5 * S, pad + bar / 2 - 5 * S, cx + 5 * S, pad + bar / 2 + 5 * S], fill=c)
    tw = d.textlength(app, font=UI_BOLD)
    if w > tw + 140 * S:
        d.text((pad + (w - tw) / 2, pad + 5 * S), app, font=UI_BOLD, fill=(220, 220, 225) if dark else (70, 70, 78))
    mask = Image.new("L", layer.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([pad, pad, pad + w, pad + h], 10 * S, fill=255)
    if not focused:  # the optional dimming of windows behind the focused one
        dim = Image.new("RGBA", layer.size, (0, 0, 0, 60))
        body = Image.alpha_composite(body, Image.composite(dim, Image.new("RGBA", layer.size), mask))
    body.putalpha(Image.composite(body.getchannel("A"), Image.new("L", layer.size, 0), mask))
    layer = Image.alpha_composite(layer, body)
    if focused:  # the optional focus border
        ImageDraw.Draw(layer).rounded_rectangle([pad - 3 * S, pad - 3 * S, pad + w + 3 * S, pad + h + 3 * S],
                                                12 * S, outline=(120, 200, 255), width=3 * S)
    if alpha < 1:
        layer.putalpha(layer.getchannel("A").point(lambda v: int(v * alpha)))
    return layer, (int(x) - pad, int(y) - pad)


def app_icon(d, cx, cy, size, app):
    accent = APPS[app][0]
    d.rounded_rectangle([cx - size / 2, cy - size / 2, cx + size / 2, cy + size / 2], size * 0.24, fill=accent)
    tw = d.textlength(app[0], font=ICON)
    d.text((cx - tw / 2, cy - 8 * S), app[0], font=ICON, fill=(255, 255, 255))


def frosted(img, box, radius, tint=(255, 255, 255, 90), alpha=1.0):
    """Blurred, tinted panel like an NSVisualEffectView."""
    box = [int(v) for v in box]
    region = img.crop(box).filter(ImageFilter.GaussianBlur(14 * S)).convert("RGBA")
    region = Image.alpha_composite(region, Image.new("RGBA", region.size, tint))
    mask = Image.new("L", region.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, region.size[0] - 1, region.size[1] - 1], radius,
                                           fill=int(255 * alpha))
    img.paste(region, box[:2], mask)


def render(desk, bg):
    img = bg.copy().convert("RGBA")
    focused = desk.focused_app()
    for app in sorted(desk.springs, key=lambda a: a == focused):
        rect = desk.springs[app].x
        if rect[1] > H or rect[1] + rect[3] < 0:
            continue
        layer, pos = window(app, rect, app == focused, desk.alpha[app])
        img.alpha_composite(layer, (max(pos[0], 0), max(pos[1], 0)),
                           (max(-pos[0], 0), max(-pos[1], 0)))
    d = ImageDraw.Draw(img)

    # menu bar with the workspace name, like the app's status item
    frosted(img, [0, 0, W * S, MENU * S], 0, (255, 255, 255, 120))
    x = 18 * S
    for i, item in enumerate([focused, "File", "Edit", "View", "Window", "Help"]):
        d.text((x, 6 * S), item, font=UI_BOLD if i == 0 else UI, fill=(25, 25, 30))
        x += d.textlength(item, font=UI_BOLD) + 16 * S
    label = f"Workspace {desk.cur}"
    tw = d.textlength(label, font=UI_BOLD)
    d.rounded_rectangle([W * S - tw - 108 * S, 4 * S, W * S - 92 * S, 22 * S], 5 * S, fill=(30, 30, 40, 200))
    d.text((W * S - tw - 100 * S, 6 * S), label, font=UI_BOLD, fill=(255, 255, 255))
    d.text((W * S - 80 * S, 6 * S), "Mon 9:41", font=UI, fill=(25, 25, 30))

    # dock
    apps = list(APPS)
    dw = len(apps) * 44 * S + 16 * S
    dx = (W * S - dw) / 2
    frosted(img, [dx, (H - 52) * S, dx + dw, (H - 6) * S], 14 * S)
    for i, app in enumerate(apps):
        app_icon(d, dx + 30 * S + i * 44 * S, (H - 29) * S, 36 * S, app)

    # minimap: one icon per column, fades out after navigating
    age = desk.t - desk.map_t
    a = 1.0 if age < 0.9 else max(0.0, 1 - (age - 0.9) / 0.4)
    if a > 0:
        cols = desk.cols()
        mw = len(cols) * 40 * S + 16 * S
        mx, my = (W * S - mw) / 2, (MENU + 18) * S
        frosted(img, [mx, my, mx + mw, my + 48 * S], 12 * S, (255, 255, 255, 110), a)
        over = Image.new("RGBA", img.size, (0, 0, 0, 0))
        od = ImageDraw.Draw(over)
        for i, (_, col_apps) in enumerate(cols):
            cx = mx + 28 * S + i * 40 * S
            if i == desk.focus[desk.cur]:
                od.rounded_rectangle([cx - 18 * S, my + 6 * S, cx + 18 * S, my + 42 * S], 8 * S,
                                     fill=(255, 255, 255, 170))
            app_icon(od, cx, my + 24 * S, 26 * S, col_apps[0])
            if len(col_apps) > 1:
                od.text((cx + 8 * S, my + 28 * S), str(len(col_apps)), font=UI_BOLD, fill=(20, 20, 20))
        over.putalpha(over.getchannel("A").point(lambda v: int(v * a)))
        img.alpha_composite(over)

    # pressed shortcut and caption
    age = desk.t - desk.key_t
    if desk.key and age < 1.6:
        a = min(1.0, age / 0.12, (1.6 - age) / 0.3)
        over = Image.new("RGBA", img.size, (0, 0, 0, 0))
        od = ImageDraw.Draw(over)
        pop = 1 + 0.08 * max(0.0, 1 - age / 0.15)  # key-down bounce
        sizes = [max(34 * S, od.textlength(k, font=KEY) + 18 * S) * pop for k in desk.key]
        cw = od.textlength(desk.caption, font=CAPTION)
        total = sum(sizes) + 8 * S * (len(sizes) - 1) + 20 * S + cw + 36 * S
        x, y = (W * S - total) / 2, (H - DOCK - 70) * S
        od.rounded_rectangle([x, y, x + total, y + 52 * S], 16 * S, fill=(18, 18, 24, 215))
        x += 18 * S
        for k, kw in zip(desk.key, sizes):
            kh = 34 * S * pop
            ky = y + 26 * S - kh / 2
            od.rounded_rectangle([x, ky + 3 * S, x + kw, ky + kh + 3 * S], 8 * S, fill=(60, 60, 70))
            od.rounded_rectangle([x, ky, x + kw, ky + kh], 8 * S, fill=(245, 245, 250))
            tw = od.textlength(k, font=KEY)
            od.text((x + (kw - tw) / 2, ky + kh / 2 - 11 * S), k, font=KEY, fill=(25, 25, 30))
            x += kw + 8 * S
        od.text((x + 12 * S, y + 17 * S), desk.caption, font=CAPTION, fill=(255, 255, 255))
        over.putalpha(over.getchannel("A").point(lambda v: int(v * a)))
        img.alpha_composite(over)

    return img.convert("RGB").resize((W, H), Image.LANCZOS)


def main():
    desk, bg = Desk(), wallpaper()
    # (seconds to hold before the step, keys, caption, action)
    script = [
        (1.0, ["⌃", "⌥", "→"], "Focus right", lambda: desk.focus_col(1)),
        (1.4, ["⌃", "⌥", "←"], "Focus left", lambda: desk.focus_col(-1)),
        (1.0, ["⌃", "⌥", "←"], "Focus left", lambda: desk.focus_col(-1)),
        (1.4, ["⌘", "N"], "New window opens as a column", lambda: desk.open_window("Code")),
        (1.6, ["⌃", "⌥", "⇧", "→"], "Move column right", lambda: desk.move_col(1)),
        (1.4, ["⌃", "⌥", "R"], "Cycle width", lambda: desk.cycle_width()),
        (1.4, ["⌃", "⌥", "R"], "Cycle width", lambda: desk.cycle_width()),
        (1.4, ["⌃", "⌥", "I"], "Stack next window", lambda: desk.consume()),
        (1.6, ["⌃", "⌥", "2"], "Workspace 2", lambda: desk.switch(2)),
        (1.6, ["⌃", "⌥", "1"], "Workspace 1", lambda: desk.switch(1)),
        (1.8, None, "", None),
    ]
    frames = []
    for hold, keys, caption, action in script:
        for _ in range(int(hold * FPS)):
            desk.tick()
            frames.append(render(desk, bg))
        if action:
            desk.press(keys, caption)
            action()
    out = sys.argv[1] if len(sys.argv) > 1 else OUT
    # ffmpeg's palettegen + bayer dithering keeps the wallpaper gradient smooth and the file small
    vf = "split[a][b];[a]palettegen=stats_mode=full[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"
    cmd = ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
           "-r", str(FPS), "-i", "-", "-vf", vf, "-loop", "0", out]
    subprocess.run(cmd, input=b"".join(f.tobytes() for f in frames), check=True)
    print(f"{out}: {len(frames)} frames, {os.path.getsize(out) // 1024} KiB")


if __name__ == "__main__":
    main()
