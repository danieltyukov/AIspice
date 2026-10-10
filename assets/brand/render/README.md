# PCB hero render

`pcb_scene.py` builds the website hero image in Blender from scratch: a close-up of
a small dark circuit board where one trace glows brand teal, runs to a castellated
pad on the board edge, and then leaves the board as a luminous tube shaped like the
logo waveform (flat, steep rise, overshoot, settle). The glowing via where the trace
starts is the dot at the start of the logo mark.

Nothing is downloaded. The board, parts, solder, silkscreen, copper, materials,
lights, camera, render settings and the post step are all in the script, so the
images can be regenerated or changed at any time.

## Output

The finished images live in `site/public/img/render/` (the square crops the script can also write are not used by the site and are not committed):

| File | Size | Use |
| --- | --- | --- |
| `pcb-light.webp`, `pcb-light.avif` | 2400 x 1200 | 2:1 band on the light page (#fbfbf9) |
| `pcb-dark.webp`, `pcb-dark.avif` | 2400 x 1200 | 2:1 band on the dark page (#0e0e10) |

The backdrop in each image is graded to exactly the page colour, so the images can
sit straight on the page with no visible edge. The post step estimates the bare
backdrop as a smooth field (masking out the board, its shadow and the glow), corrects
it to the page colour, and snaps anything within a few levels of it exactly onto it.
Shadows, reflections and glow spill stay, but anything that still differs from the
page colour is tapered to nothing over the outer 12%, so all four edges are flat page
colour. Keep the subject inside that margin if you change the framing.

The 16-bit PNG masters are large and are not kept in git. Regenerate them with the
commands below.

## Requirements

- Blender 5.2 (`blender` on the PATH). It uses Cycles on the CPU; a GPU is not needed.
- For `--web`: `python3` with Pillow (WebP) and ImageMagick with AVIF support
  (`magick` or `convert`). WebP goes through Pillow because ImageMagick 6 ignores
  the WebP quality setting.

The WebP files are lossless (about 170 to 320 KB). Lossy WebP stores colour as YUV
and decodes #fbfbf9 as (250, 251, 248) even at quality 100, which would leave a seam
against the page. AVIF is lossy at quality 90 with full-resolution chroma and does
reproduce both page colours exactly. If you change the page colours, update
`PAGE_BG` in the script and rerun with `--post-only`.

## Regenerating

Run from the repository root. Each final takes about 4 to 5 minutes on a
22-thread laptop CPU at 320 samples.

```sh
OUT=/tmp/pcb   # masters go here, outside the repo
WEB=site/public/img/render

blender -b -P assets/brand/render/pcb_scene.py -- --variant light --frame band \
    --res 2400x1200 --out $OUT/pcb-light.png --web $WEB
blender -b -P assets/brand/render/pcb_scene.py -- --variant dark --frame band \
    --res 2400x1200 --out $OUT/pcb-dark.png --web $WEB
blender -b -P assets/brand/render/pcb_scene.py -- --variant light --frame square \
    --res 1600x1600 --out $OUT/pcb-light-square.png --web $WEB
blender -b -P assets/brand/render/pcb_scene.py -- --variant dark --frame square \
    --res 1600x1600 --out $OUT/pcb-dark-square.png --web $WEB
```

Each run writes `NAME.png` (the raw 16-bit render), `NAME-final.png` (graded to the
page colour) and an 8-bit copy used for encoding, then the WebP and AVIF files into
the `--web` folder. To change only the grading or encoding, rerun the same command
with `--post-only`; it reuses `NAME.png` and takes about half a minute.

For a quick framing check, render small with few samples:

```sh
blender -b -P assets/brand/render/pcb_scene.py -- --variant dark --res 960x480 \
    --samples 24 --out /tmp/pcb/preview.png
```

To look at detail at full size without rendering the whole frame, add
`--crop 0.42,0.22,0.75,0.62` (a region of the frame as fractions, origin at the
bottom left). To try a different camera without editing the file, pass for example
`--cam "az=-60;el=24;dist=180"`. All flags are listed in the script's docstring.

## What to edit

- Camera framings: `FRAMES` (target point, azimuth, elevation, distance, lens,
  f-number, focus point). The f-number is photographic; the script converts it for
  the millimetre scene.
- Lights and background brightness: `build_lights`, `build_world`, `mat_floor`, and
  the exposure in `setup_render` (the light variant runs half a stop brighter).
- Glow: `mat_glow` (colour, core whitening) and its strength in `build_materials`;
  bloom size and strength in `setup_compositor`. Bloom only comes from directly
  visible emission, so the gold and the highlights do not bloom.
- Board layout: `build_layout` places the parts and draws the copper that shows
  faintly through the mask. `build_signal` builds the glowing trace and the waveform
  tube from the logo path in `WAVE`; `WAVE_S` sets its size in millimetres per logo
  unit and `TUBE_R` its thickness.
- Mask colour: `mat_mask`.

## Notes

- The scene is modelled at real scale in millimetres: 34 x 22 mm board, 1.2 mm
  thick, castellated edges on a 2.54 mm pitch, an SOIC-8, an SOT-23, 0603 resistors
  and capacitors, a 5 mm aluminium electrolytic, an unpopulated 0603 footprint with
  bare ENIG pads, a test point and two fiducials.
- The silkscreen uses a small single-stroke font defined in the script, so no font
  file is needed. Only reference designators are printed; there are no logos or
  part numbers.
- The view transform is Khronos PBR Neutral, which keeps base colours accurate and
  rolls bright emission off toward white instead of clipping.
