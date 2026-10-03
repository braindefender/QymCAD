import os
import re
import urllib.request
import zipfile
import io

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ID_RS = os.path.join(ROOT, "crates", "qymcad-ui-state", "src", "icons", "id.rs")
OUT_QICONS = os.path.join(ROOT, "assets", "icon-themes", "default.qicons")

# 1. Parse icon definitions from id.rs
# Format: `category / name : Variant => ph::GLYPH_NAME,`
pattern = re.compile(r'(\w+)\s*/\s*(r#\w+|\w+)\s*:\s*(\w+)\s*=>\s*ph::(\w+)')

icons = []
with open(ID_RS, "r", encoding="utf-8") as f:
    for line in f:
        m = pattern.search(line)
        if m:
            cat, name, variant, glyph = m.groups()
            clean_name = name.removeprefix("r#")
            icons.append((cat, clean_name, variant, glyph))

print(f"Parsed {len(icons)} icon mappings from id.rs")

# 2. Map Phosphor glyph identifier (e.g. LINE_SEGMENT) to official SVG filename (e.g. line-segment.svg)
# Phosphor naming convention: snake_case / kebab-case
# Note: In egui-phosphor, name is uppercase with underscores, original name had hyphens
def glyph_to_svg_name(g):
    return g.lower().replace("_", "-") + ".svg"

# 3. Download SVGs from phosphor-icons/core repo
# URL: https://raw.githubusercontent.com/phosphor-icons/core/main/assets/regular/{name}.svg
BASE_URL = "https://raw.githubusercontent.com/phosphor-icons/core/main/assets/regular/"

# Cache downloaded SVGs to avoid redundant downloads
svg_cache = {}
unique_glyphs = set(g for _, _, _, g in icons)
print(f"Total unique Phosphor glyphs to fetch: {len(unique_glyphs)}")

for g in sorted(unique_glyphs):
    svg_name = glyph_to_svg_name(g)
    url = BASE_URL + svg_name
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            content = resp.read()
            svg_cache[g] = content
            print(f"  Downloaded {svg_name} ({len(content)} bytes)")
    except Exception as e:
        print(f"  ERROR downloading {svg_name} from {url}: {e}")
        raise e

# 4. Prepare default.qicons zip archive
os.makedirs(os.path.dirname(OUT_QICONS), exist_ok=True)

manifest_ron = """(
    package_type: IconTheme,
    id: "default",
    name: "QymCAD Default",
    version: "1.0.0",
    author: "Phosphor Icons & QymCAD Team",
    license: "MIT",
    description: "Built-in default monochrome SVG vector icons based on Phosphor",
    color_mode: Monochrome,
)
"""

with zipfile.ZipFile(OUT_QICONS, "w", zipfile.ZIP_DEFLATED) as zf:
    zf.writestr("manifest.ron", manifest_ron.strip())
    
    for cat, name, variant, glyph in icons:
        svg_data = svg_cache[glyph]
        # Verify SVG validity (viewBox present, no raster)
        text = svg_data.decode("utf-8")
        assert "<svg" in text and "viewBox" in text, f"Invalid SVG for {cat}/{name}"
        assert "<image" not in text and "data:image/" not in text, f"Raster in {cat}/{name}"
        
        path_in_zip = f"icons/{cat}/{name}.svg"
        zf.writestr(path_in_zip, svg_data)
        print(f"  Packaged {path_in_zip} (from {glyph})")

print(f"\n[+] Successfully generated {OUT_QICONS} with {len(icons)} SVG icons!")
print(f"Archive size: {os.path.getsize(OUT_QICONS)} bytes")
