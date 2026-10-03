# Default Vector Icon Theme

The built-in, monochrome vector icon set for **QymCAD**.

## Overview
- **ID**: `default`
- **Color Mode**: `Monochrome` (adapts dynamically to UI text color across light and dark color schemes)
- **Coverage**: 100% (107/107 CAD icons)
- **License**: MIT / Apache-2.0
- **Base Layer**: Yes (infallible fallback cascade anchor)

## Characteristics
- **ViewBox**: Standard 64×64 square viewport.
- **Visual Style**: Clean, modern geometric outlines and strokes based on Phosphor design guidelines.
- **Adaptive Contrast**: SVGs use `currentColor` or omission of hardcoded fills so that icons remain crisp and high-contrast in all color themes.
- **Fallbacks**: If higher-priority themes in the cascade do not define a specific tool icon, CAD seamlessly renders the corresponding icon from this theme.
