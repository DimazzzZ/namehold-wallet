#!/bin/sh
# Renders background.svg into background.png at 2x (1320x800) tagged 144 dpi,
# so Finder shows it at 660x400 points and sharp on Retina screens.
# Needs rsvg-convert and ImageMagick (brew install librsvg imagemagick).
set -eu
cd "$(dirname "$0")"
rsvg-convert -w 1320 -h 800 background.svg -o background.tmp.png
magick background.tmp.png -strip background.png
sips -s dpiWidth 144 -s dpiHeight 144 background.png >/dev/null
rm background.tmp.png
