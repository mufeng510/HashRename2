#!/usr/bin/env python3
"""生成 HashRename 应用图标底图(1024x1024 PNG)。
仅供构建工具链使用,不是产品核心代码。"""
from PIL import Image, ImageDraw, ImageFont
import sys

SIZE = 1024
img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# 圆角方块背景(深靛蓝渐变模拟:两块叠加)
bg = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
bd = ImageDraw.Draw(bg)
for i in range(SIZE):
    t = i / SIZE
    r = int(30 + (44 - 30) * t)
    g = int(41 + (62 - 41) * t)
    b = int(59 + (80 - 59) * t)
    bd.line([(0, i), (SIZE, i)], fill=(r, g, b, 255))

# 圆角遮罩
mask = Image.new("L", (SIZE, SIZE), 0)
md = ImageDraw.Draw(mask)
md.rounded_rectangle([16, 16, SIZE - 16, SIZE - 16], radius=200, fill=255)
img.paste(bg, (0, 0), mask)

# 哈希条(象征文件内容指纹)
bar_color = (148, 163, 184, 255)
bars = [(230, 120, 560), (230, 175, 760), (230, 230, 430), (230, 285, 660)]
for x0, y0, x1 in bars:
    d.rounded_rectangle([x0, y0, x1, y0 + 26], radius=13, fill=bar_color)

# "001" 序号主文字
teal = (45, 212, 191, 255)
font = None
for fp in [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc",
]:
    try:
        font = ImageFont.truetype(fp, 340)
        break
    except Exception:
        pass
if font is None:
    font = ImageFont.load_default()

text = "001"
bbox = d.textbbox((0, 0), text, font=font)
tw = bbox[2] - bbox[0]
th = bbox[3] - bbox[1]
tx = (SIZE - tw) // 2 - bbox[0]
ty = 430 - bbox[1]
d.text((tx, ty), text, font=font, fill=teal)

# 底部小箭头(重命名循环)
d.rounded_rectangle([230, 850, 700, 876], radius=13, fill=teal)
d.polygon([(700, 830), (794, 863), (700, 896)], fill=teal)

out = sys.argv[1] if len(sys.argv) > 1 else "app-icon.png"
img.save(out, "PNG")
print(f"written {out}")
