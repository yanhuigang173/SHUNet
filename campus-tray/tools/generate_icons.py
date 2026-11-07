"""Export code-native vector artwork inspired by the supplied tray design.

Development tool only (Pillow); the executable embeds the finished ICO files.
No pixels from the reference screenshot are read or modified.
"""
from pathlib import Path
import math
import struct
from PIL import Image, ImageDraw, ImageFont

ASSETS = Path(__file__).resolve().parents[1] / "assets" / "icons"
ASSETS.mkdir(parents=True, exist_ok=True)
STATES = [
    ("disconnected", "未连接", "#5D6979", None),
    ("connected", "基础状态", "#008FFF", None),
    ("authenticated", "已认证", "#00B84A", "check"),
    ("authenticating", "正在认证", "#FF8800", "spinner"),
    ("error", "网络异常", "#F10B22", "alert"),
]
SIZES = (16, 20, 24, 32, 48, 64)


class PathShape:
    def __init__(self):
        self.svg = []
        self.points = []
        self.current = (0, 0)

    def move(self, x, y):
        self.svg.append(f"M{x} {y}")
        self.current = (x, y)
        self.points.append(self.current)
        return self

    def line(self, x, y):
        self.svg.append(f"L{x} {y}")
        self.current = (x, y)
        self.points.append(self.current)
        return self

    def curve(self, x1, y1, x2, y2, x, y):
        self.svg.append(f"C{x1} {y1} {x2} {y2} {x} {y}")
        x0, y0 = self.current
        for i in range(1, 41):
            t = i / 40
            u = 1 - t
            self.points.append((u**3*x0 + 3*u*u*t*x1 + 3*u*t*t*x2 + t**3*x,
                                u**3*y0 + 3*u*u*t*y1 + 3*u*t*t*y2 + t**3*y))
        self.current = (x, y)
        return self

    def quad(self, x1, y1, x, y):
        self.svg.append(f"Q{x1} {y1} {x} {y}")
        x0, y0 = self.current
        for i in range(1, 41):
            t = i / 40
            u = 1 - t
            self.points.append((u*u*x0 + 2*u*t*x1 + t*t*x,
                                u*u*y0 + 2*u*t*y1 + t*t*y))
        self.current = (x, y)
        return self


OUTER = (PathShape().move(4, 53).line(18, 60).quad(23, 62, 23, 68)
         .line(23, 101).curve(23, 116, 32, 125, 50, 125)
         .curve(68, 125, 77, 116, 77, 101).line(77, 68)
         .quad(77, 62, 84, 58).line(96, 52).line(96, 101)
         .curve(96, 125, 80, 140, 50, 140).curve(20, 140, 4, 125, 4, 101))
STEM = (PathShape().move(30, 65).curve(36, 68, 40, 70, 40, 78)
        .line(40, 98).curve(36, 93, 31, 96, 31, 100))
SWAN = (PathShape().move(33, 102).curve(35, 96, 39, 100, 40, 105)
        .curve(42, 111, 47, 108, 47, 102).curve(47, 91, 45, 84, 51, 75)
        .curve(56, 66, 65, 61, 76, 59).curve(72, 65, 71, 70, 73, 75)
        .curve(65, 78, 66, 87, 64, 97).curve(62, 116, 42, 122, 33, 107))
DOT = (PathShape().move(41, 50).quad(50, 43, 59, 50).quad(62, 52, 59, 55)
       .line(53, 61).quad(50, 64, 47, 61).line(41, 55).quad(38, 52, 41, 50))
ARCS = [PathShape().move(12, 23).quad(50, -7, 88, 23),
        PathShape().move(27, 38).quad(50, 20, 73, 38)]


def artwork(color, badge, size, querying=False):
    scale = size * 8 / 144
    # The revised reference uses a much larger query badge. Shift the logo left
    # inside the same icon canvas so the complete white rim remains visible.
    offset_x = 6 if querying else 20
    canvas = Image.new("RGBA", (size * 8, size * 8))
    draw = ImageDraw.Draw(canvas)
    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 144 144">',
           f'<g transform="translate({offset_x} 0)" fill="{color}">']

    def xy(point):
        return ((point[0] + offset_x) * scale, point[1] * scale)

    def stroke(points, width, fill):
        points = [xy(p) for p in points]
        draw.line(points, fill=fill, width=max(1, round(width * scale)), joint="curve")
        r = width * scale / 2
        for x, y in [points[0], points[-1]]:
            draw.ellipse((x-r, y-r, x+r, y+r), fill=fill)

    for arc, width in zip(ARCS, [12, 11]):
        stroke(arc.points, width, color)
        svg.append(f'<path d="{" ".join(arc.svg)}" fill="none" stroke="{color}" stroke-width="{width}" stroke-linecap="round"/>')
    for shape in [DOT, OUTER, STEM, SWAN]:
        draw.polygon([xy(p) for p in shape.points], fill=color)
        svg.append(f'<path d="{" ".join(shape.svg)} Z"/>')
    if badge or querying:
        # Query variants replace the status badge with a larger dotted ring.
        # Keeping its colour preserves the last confirmed connection state.
        x, y, r = (90, 104, 35) if querying else (100, 119, 18)
        rim = 4 if querying else 3
        for radius, fill in [(r+rim, "white"), (r, color)]:
            x0, y0 = xy((x-radius, y-radius))
            x1, y1 = xy((x+radius, y+radius))
            draw.ellipse((x0, y0, x1, y1), fill=fill)
        svg.append(f'<circle cx="{x}" cy="{y}" r="{r+rim}" fill="white"/><circle cx="{x}" cy="{y}" r="{r}" fill="{color}"/>')
        if querying:
            for i in range(8):
                angle = (i - 2) * math.tau / 8
                opacity = 0.55 + 0.45 * i / 7
                dot_x, dot_y = x + math.cos(angle)*21, y + math.sin(angle)*21
                dx, dy = xy((dot_x, dot_y))
                dot_radius = 4.7
                radius = dot_radius * scale
                rgb = tuple(int(color[j:j+2], 16) for j in (1, 3, 5))
                fill = tuple(round(channel*(1-opacity) + 255*opacity) for channel in rgb)
                draw.ellipse((dx-radius, dy-radius, dx+radius, dy+radius), fill=fill)
                svg.append(f'<circle cx="{dot_x}" cy="{dot_y}" r="{dot_radius}" fill="white" fill-opacity="{opacity:.3f}"/>')
        elif badge == "check":
            points = [(x-8, y), (x-2, y+6), (x+9, y-6)]
            stroke(points, 4, "white")
            svg.append(f'<path d="M{x-8} {y} L{x-2} {y+6} L{x+9} {y-6}" fill="none" stroke="white" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>')
        elif badge == "spinner":
            for i in range(8):
                angle = i * math.tau / 8
                points = [(x + math.cos(angle)*radius, y + math.sin(angle)*radius) for radius in (8, 12)]
                stroke(points, 2.8, "white")
                a, b = points
                svg.append(f'<path d="M{a[0]} {a[1]} L{b[0]} {b[1]}" stroke="white" stroke-width="2.8" stroke-linecap="round"/>')
        else:
            stroke([(x, y-10), (x, y+1)], 4.2, "white")
            dx, dy = xy((x, y+9))
            radius = 2.2 * scale
            draw.ellipse((dx-radius, dy-radius, dx+radius, dy+radius), fill="white")
            svg.append(f'<path d="M{x} {y-10} L{x} {y+1}" stroke="white" stroke-width="4.2" stroke-linecap="round"/><circle cx="{x}" cy="{y+9}" r="2.2" fill="white"/>')
    svg.append('</g></svg>')
    return canvas.resize((size, size), Image.Resampling.LANCZOS), "\n".join(svg)


def icon_bitmap(image):
    size = image.width
    rgba = image.load()
    pixels = bytearray()
    mask_stride = ((size + 31) // 32) * 4
    mask = bytearray(mask_stride * size)
    for row, y in enumerate(range(size - 1, -1, -1)):
        for x in range(size):
            r, g, b, a = rgba[x, y]
            pixels += bytes((b, g, r, a))
            if a == 0:
                mask[row*mask_stride + x//8] |= 0x80 >> (x % 8)
    header = struct.pack('<IiiHHIIiiII', 40, size, size*2, 1, 32, 0, len(pixels)+len(mask), 0, 0, 0, 0)
    return header + pixels + mask


def export_ico(name, images):
    chunks = [icon_bitmap(image) for image in images]
    data = bytearray(struct.pack('<HHH', 0, 1, len(images)))
    offset = 6 + 16*len(images)
    for image, chunk in zip(images, chunks):
        size = image.width
        data += struct.pack('<BBBBHHII', size if size < 256 else 0, size if size < 256 else 0, 0, 0, 1, 32, len(chunk), offset)
        offset += len(chunk)
    (ASSETS / f'{name}.ico').write_bytes(data + b''.join(chunks))


preview = Image.new('RGB', (1100, 830), '#f6f8fc')
pen = ImageDraw.Draw(preview)
font_path = Path('C:/Windows/Fonts/msyh.ttc')
font = ImageFont.truetype(str(font_path), 20) if font_path.exists() else ImageFont.load_default()
small = ImageFont.truetype(str(font_path), 13) if font_path.exists() else ImageFont.load_default()
for index, (name, label, color, badge) in enumerate(STATES):
    images = [artwork(color, badge, size)[0] for size in SIZES]
    large, svg = artwork(color, badge, 160)
    (ASSETS / f'{name}.svg').write_text(svg, encoding='utf-8')
    artwork(color, badge, 64)[0].save(ASSETS / f'{name}.png')
    export_ico(name, images)
    query_images = [artwork(color, badge, size, querying=True)[0] for size in SIZES]
    query_large, query_svg = artwork(color, badge, 160, querying=True)
    (ASSETS / f'{name}-querying.svg').write_text(query_svg, encoding='utf-8')
    artwork(color, badge, 64, querying=True)[0].save(ASSETS / f'{name}-querying.png')
    export_ico(f'{name}-querying', query_images)
    x = index * 220
    preview.paste(large, (x+30, 25), large)
    pen.text((x+110, 207), label, fill=color, font=font, anchor='mm')
    for y, background in [(250, '#ffffff'), (342, '#202733')]:
        pen.rounded_rectangle((x+10, y, x+210, y+77), radius=8, fill=background)
        for position, size in enumerate((16, 20, 24, 32)):
            icon = images[SIZES.index(size)]
            center = x + 35 + position*45
            preview.paste(icon, (center-size//2, y+27-size//2), icon)
            pen.text((center, y+59), str(size), fill='#8994a5', font=small, anchor='mm')
    preview.paste(query_large, (x+30, 455), query_large)
    pen.text((x+110, 637), f'{label} · 查询中', fill=color, font=font, anchor='mm')
    for y, background in [(670, '#ffffff'), (752, '#202733')]:
        pen.rounded_rectangle((x+10, y, x+210, y+69), radius=8, fill=background)
        for position, size in enumerate((16, 20, 24, 32)):
            icon = query_images[SIZES.index(size)]
            center = x + 35 + position*45
            preview.paste(icon, (center-size//2, y+23-size//2), icon)
            pen.text((center, y+53), str(size), fill='#8994a5', font=small, anchor='mm')
preview.save(ASSETS / 'preview.png')
print(f'Exported {len(STATES)} states with matching query variants and {2*len(STATES)*len(SIZES)} ICO sizes to {ASSETS}')
