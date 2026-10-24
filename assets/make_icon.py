"""Sonora のアイコンを生成する（python assets/make_icon.py）。
夜空のような紺〜紫の角丸タイルに、シアン〜マゼンタの波形（イコライザー）バーを並べたデザイン。
出力: assets/sonora.png (256px, ウィンドウ用) / assets/sonora.ico (16〜256px, EXE 用)
"""
from PIL import Image, ImageDraw, ImageFilter, ImageChops
import os

S = 1024  # 作画サイズ（縮小して滑らかにする）
OUT = os.path.dirname(os.path.abspath(__file__))


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def gradient(size, c0, c1, diagonal=False):
    w, h = size
    img = Image.new("RGB", size)
    px = img.load()
    for y in range(h):
        for x in range(w):
            t = (x + y) / (w + h) if diagonal else y / (h - 1)
            px[x, y] = lerp(c0, c1, t)
    return img


def rounded_mask(size, box, radius):
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius=radius, fill=255)
    return m


# --- 背景タイル（紺 → 紫） ---
margin = 40
tile_box = (margin, margin, S - margin, S - margin)
bg = gradient((S, S), (9, 12, 38), (74, 29, 150), diagonal=True).convert("RGBA")
tile_mask = rounded_mask((S, S), tile_box, 230)

# 中央のやわらかい光（バーの後ろ）
glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
gd = ImageDraw.Draw(glow)
gd.ellipse((S * 0.18, S * 0.22, S * 0.82, S * 0.78), fill=(56, 189, 248, 120))
glow = glow.filter(ImageFilter.GaussianBlur(120))
bg = Image.alpha_composite(bg, glow)

# --- 波形バー ---
heights = [0.30, 0.52, 0.78, 1.00, 0.66, 0.46, 0.26]
bar_w, gap = 76, 44
total_w = len(heights) * bar_w + (len(heights) - 1) * gap
x0 = (S - total_w) // 2
max_h = 560
cy = S // 2 + 6

bars = Image.new("RGBA", (S, S), (0, 0, 0, 0))
for i, hgt in enumerate(heights):
    h = int(max_h * hgt)
    x = x0 + i * (bar_w + gap)
    box = (x, cy - h // 2, x + bar_w, cy + h // 2)
    # 1本ごとにシアン（上）→ マゼンタ（下）のグラデーション
    g = gradient((bar_w, h), (34, 211, 238), (232, 121, 249)).convert("RGBA")
    m = rounded_mask((bar_w, h), (0, 0, bar_w - 1, h - 1), bar_w // 2)
    bars.paste(g, (box[0], box[1]), m)

# バーの下に薄い影、上に発光
shadow = bars.filter(ImageFilter.GaussianBlur(26))
shadow = ImageChops.multiply(shadow, Image.new("RGBA", (S, S), (255, 255, 255, 150)))
bg = Image.alpha_composite(bg, shadow)
bg = Image.alpha_composite(bg, bars)

# --- ガラスのような上面のハイライトと縁 ---
hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
hd = ImageDraw.Draw(hl)
hd.rounded_rectangle((margin + 14, margin + 14, S - margin - 14, S // 2), radius=216, fill=(255, 255, 255, 26))
bg = Image.alpha_composite(bg, hl)
rim = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(rim).rounded_rectangle(tile_box, radius=230, outline=(255, 255, 255, 60), width=6)
bg = Image.alpha_composite(bg, rim)

# タイルの形で切り抜く
final = Image.new("RGBA", (S, S), (0, 0, 0, 0))
final.paste(bg, (0, 0), tile_mask)

png = final.resize((256, 256), Image.LANCZOS)
png.save(os.path.join(OUT, "sonora.png"))
sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
def dib(img):
    """ICO 用の無圧縮 32bit DIB（BMP）データ。PNG 圧縮を使わない（リソースの乱雑さを抑え、ウイルス対策ソフトに「パック済み」と誤認されにくくする）"""
    import struct
    w, h = img.size
    px = img.convert("RGBA").load()
    rows = bytearray()
    for y in range(h - 1, -1, -1):  # BMP は下の行から
        for x in range(w):
            r, g, b, a = px[x, y]
            rows += bytes((b, g, r, a))
    mask_row = ((w + 31) // 32) * 4
    and_mask = bytes(mask_row * h)  # アルファを使うので全部 0
    head = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, len(rows) + len(and_mask), 0, 0, 0, 0)
    return head + bytes(rows) + and_mask


import struct
frames = [(sz, dib(final.resize((sz, sz), Image.LANCZOS))) for sz in (16, 24, 32, 48, 64, 128, 256)]
out = bytearray(struct.pack("<HHH", 0, 1, len(frames)))
offset = 6 + 16 * len(frames)
for sz, data in frames:
    out += struct.pack("<BBBBHHII", sz % 256, sz % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
for _, data in frames:
    out += data
open(os.path.join(OUT, "sonora.ico"), "wb").write(out)
print("ok")
