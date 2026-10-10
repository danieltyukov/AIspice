"""aispice hero render: a close-up of a small circuit board whose output trace
glows teal and lifts off the board edge as the brand step-response waveform.

Everything (geometry, materials, lights, camera, render and post settings) is
built from this file, so the images can be regenerated at any time. Units are
millimetres. Rendering uses Cycles with OpenImageDenoise.

Run headless from the repository root:

    blender -b -P assets/brand/render/pcb_scene.py -- \
        --variant light --frame band --res 2400x1200 --samples 320 \
        --out /tmp/pcb/pcb-light.png --web site/public/img/render

Flags (all after the bare "--"):

    --variant light|dark   light: off-white page background (#fbfbf9) with a soft
                           contact shadow. dark: near-black background (#0e0e10).
    --frame band|square    camera framing. band is the 2:1 hero, square the 1:1 crop.
                           Default: band if the image is wider than tall, else square.
    --res WxH              output size in pixels (default 2400x1200).
    --samples N            Cycles samples per pixel (default 320). 16 to 32 is enough
                           for a framing preview.
    --out PATH             PNG to write. PATH is the 16-bit master straight from the
                           render; PATH with "-final" added is the graded copy whose
                           backdrop is exactly the page colour, so it sits on the
                           page with no visible edge.
    --web DIR              also encode the graded copy to DIR/<name>.webp (Pillow,
                           run through python3) and DIR/<name>.avif (ImageMagick).
                           <name> is the --out file name without extension.
                           WebP is lossless by default: lossy WebP stores colour as
                           YUV and cannot reproduce #fbfbf9 exactly.
    --webp-q N             encode WebP lossy at quality N instead.
    --avif-q N             AVIF quality (default 90, full-resolution chroma).
    --blend PATH           save the built scene as a .blend file for inspection.
    --threads N            CPU render threads (default: all).
    --crop X0,Y0,X1,Y1     render only this region (fractions of the frame, origin
                           bottom left) for inspecting detail at full resolution.
                           Skips the post step.
    --post-only            skip building and rendering; grade and encode the existing
                           master at --out again (after changing the post step).
    --cam "k=v;k=v"        override camera fields of the chosen frame while trying
                           framings, e.g. --cam "az=-60;el=24;dist=180;target=11,-2,6".
                           Keys: target, az, el, dist, lens, fstop, focus, shift.
"""

import argparse
import math
import os
import shutil
import subprocess
import sys
import time

import bpy
import bmesh
import numpy as np
from mathutils import Vector


# ---------------------------------------------------------------- arguments

def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser(prog="pcb_scene.py")
    p.add_argument("--variant", choices=["light", "dark"], default="light")
    p.add_argument("--frame", choices=["band", "square"], default=None)
    p.add_argument("--res", default="2400x1200")
    p.add_argument("--samples", type=int, default=320)
    p.add_argument("--out", default="/tmp/pcb-render.png")
    p.add_argument("--web", default=None)
    p.add_argument("--webp-q", type=int, default=None)
    p.add_argument("--avif-q", type=int, default=90)
    p.add_argument("--blend", default=None)
    p.add_argument("--threads", type=int, default=0)
    p.add_argument("--crop", default=None)
    p.add_argument("--cam", default=None)
    p.add_argument("--post-only", action="store_true")
    a = p.parse_args(argv)
    a.w, a.h = (int(v) for v in a.res.lower().split("x"))
    if a.frame is None:
        a.frame = "band" if a.w > a.h else "square"
    return a


ARGS = parse_args()
DARK = ARGS.variant == "dark"

# ---------------------------------------------------------------- constants

BOARD_W, BOARD_H, BOARD_T = 34.0, 22.0, 1.2   # board outline and thickness
ZT = BOARD_T                                  # top of the solder mask
XE = BOARD_W / 2                              # right board edge (castellated)
CAST_R = 0.4                                  # castellated half-hole radius
CAST_Y = [-8.89, -6.35, -3.81, -1.27, 1.27, 3.81, 6.35, 8.89]
SIG_Y = -3.81                                 # row of the glowing signal
WAVE_S = 1.3                                  # mm per unit of the 32 px logo grid
TUBE_R = 0.22                                 # radius of the luminous tube
PAD_H = 0.045                                 # copper plus ENIG above the mask
PAGE_BG = "#0e0e10" if DARK else "#fbfbf9"     # site page backgrounds

# The logo waveform on its 32x32 grid (assets/brand/mark.svg).
WAVE = [
    ((6.0, 21.0), (10.2, 21.0)),
    ((10.2, 21.0), (12.4, 21.0), (12.6, 7.8), (15.2, 7.8)),
    ((15.2, 7.8), (17.6, 7.8), (17.5, 13.6), (19.8, 13.6)),
    ((19.8, 13.6), (21.7, 13.6), (21.8, 10.1), (23.5, 10.1)),
    ((23.5, 10.1), (24.9, 10.1), (25.3, 11.0), (26.5, 11.0)),
]


def hex_lin(h):
    h = h.lstrip("#")
    c = [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    return tuple(v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in c)


# ---------------------------------------------------------------- scene reset

bpy.ops.wm.read_factory_settings(use_empty=True)
SCENE = bpy.context.scene
SCENE.unit_settings.system = "METRIC"
SCENE.unit_settings.scale_length = 0.001
SCENE.unit_settings.length_unit = "MILLIMETERS"
COLL = SCENE.collection


# ---------------------------------------------------------------- mesh helpers

def mesh_obj(name, verts, faces, mats, mat_idx=None, smooth=True, sharp_deg=30.0,
             parent=None, recalc=True):
    me = bpy.data.meshes.new(name)
    me.from_pydata([tuple(v) for v in verts], [], [list(f) for f in faces])
    if mat_idx is not None:
        me.polygons.foreach_set("material_index", list(mat_idx))
    me.validate()
    if recalc:
        bm = bmesh.new()
        bm.from_mesh(me)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(me)
        bm.free()
    for m in mats:
        me.materials.append(m)
    me.polygons.foreach_set("use_smooth", [smooth] * len(me.polygons))
    if smooth and sharp_deg is not None:
        me.set_sharp_from_angle(angle=math.radians(sharp_deg))
    ob = bpy.data.objects.new(name, me)
    COLL.objects.link(ob)
    if parent is not None:
        ob.parent = parent
    return ob


def empty(name, loc, rot_deg=0.0):
    ob = bpy.data.objects.new(name, None)
    ob.location = loc
    ob.rotation_euler = (0, 0, math.radians(rot_deg))
    COLL.objects.link(ob)
    return ob


def add_bevel(ob, width, seg=3, angle=40.0):
    m = ob.modifiers.new("bevel", "BEVEL")
    m.width = width
    m.segments = seg
    m.limit_method = "ANGLE"
    m.angle_limit = math.radians(angle)
    m.harden_normals = True
    return m


def prism(poly, z0, z1, side_mat=None):
    """Extrude a CCW polygon. Returns verts, faces, material index per face."""
    n = len(poly)
    V = [(x, y, z0) for x, y in poly] + [(x, y, z1) for x, y in poly]
    F = [list(range(n - 1, -1, -1)), list(range(n, 2 * n))]
    M = [0, 0]
    for i in range(n):
        j = (i + 1) % n
        F.append([i, j, n + j, n + i])
        M.append(side_mat[i] if side_mat else 0)
    return V, F, M


def rrect(w, h, r, seg=8, cx=0.0, cy=0.0):
    pts = []
    for sx, sy, a0 in ((1, -1, -90), (1, 1, 0), (-1, 1, 90), (-1, -1, 180)):
        ccx, ccy = cx + sx * (w / 2 - r), cy + sy * (h / 2 - r)
        for k in range(seg + 1):
            a = math.radians(a0 + 90 * k / seg)
            pts.append((ccx + r * math.cos(a), ccy + r * math.sin(a)))
    return pts


def circle_pts(r, n=32, cx=0.0, cy=0.0):
    return [(cx + r * math.cos(2 * math.pi * k / n), cy + r * math.sin(2 * math.pi * k / n))
            for k in range(n)]


def ring_stack(name, rings, mat, parent=None, bevel=0.0, cx=0.0, cy=0.0):
    """Box-like solid through rectangular rings [(sx, sy, z), ...] bottom to top."""
    V, F = [], []
    for sx, sy, z in rings:
        V += [(cx - sx / 2, cy - sy / 2, z), (cx + sx / 2, cy - sy / 2, z),
              (cx + sx / 2, cy + sy / 2, z), (cx - sx / 2, cy + sy / 2, z)]
    F.append([3, 2, 1, 0])
    k = len(rings) - 1
    F.append([4 * k, 4 * k + 1, 4 * k + 2, 4 * k + 3])
    for r in range(k):
        for i in range(4):
            j = (i + 1) % 4
            F.append([4 * r + i, 4 * r + j, 4 * (r + 1) + j, 4 * (r + 1) + i])
    ob = mesh_obj(name, V, F, [mat], parent=parent, sharp_deg=25)
    if bevel > 0:
        add_bevel(ob, bevel, 3, 25)
    return ob


def box(name, size, center, mat, parent=None, bevel=0.0, seg=3):
    sx, sy, sz = size
    cx, cy, cz = center
    return ring_stack(name, [(sx, sy, cz - sz / 2), (sx, sy, cz + sz / 2)], mat,
                      parent=parent, bevel=bevel, cx=cx, cy=cy)


def cylinder(name, r, z0, z1, mat, cx=0.0, cy=0.0, n=40, parent=None, bevel=0.0):
    V, F, M = prism(circle_pts(r, n, cx, cy), z0, z1)
    ob = mesh_obj(name, V, F, [mat], parent=parent, sharp_deg=40)
    if bevel > 0:
        add_bevel(ob, bevel, 3, 50)
    return ob


def lathe(name, prof, mats, seg_mat=None, n=64, cx=0.0, cy=0.0, parent=None, sharp=35):
    """Revolve a profile [(r, z), ...] around a vertical axis. r == 0 ends are poles."""
    V, F, M = [], [], []
    idx = []
    for r, z in prof:
        if r <= 1e-9:
            idx.append([len(V)])
            V.append((cx, cy, z))
        else:
            ring = []
            for k in range(n):
                a = 2 * math.pi * k / n
                ring.append(len(V))
                V.append((cx + r * math.cos(a), cy + r * math.sin(a), z))
            idx.append(ring)
    for s in range(len(prof) - 1):
        a, b = idx[s], idx[s + 1]
        mi = seg_mat[s] if seg_mat else 0
        for k in range(n):
            k2 = (k + 1) % n
            if len(a) == 1:
                F.append([a[0], b[k], b[k2]])
            elif len(b) == 1:
                F.append([a[k], a[k2], b[0]])
            else:
                F.append([a[k], a[k2], b[k2], b[k]])
            M.append(mi)
    return mesh_obj(name, V, F, mats, mat_idx=M, parent=parent, sharp_deg=sharp)


def fillet_path(pts, r, n=10):
    """Round the inner corners of a 3D polyline with arcs of radius r."""
    pts = [Vector(p) for p in pts]
    out = [pts[0]]
    for i in range(1, len(pts) - 1):
        a, b, c = pts[i - 1], pts[i], pts[i + 1]
        d1, d2 = (a - b).normalized(), (c - b).normalized()
        ang = d1.angle(d2)
        if ang > math.pi - 1e-3:
            out.append(b)
            continue
        t = r / math.tan(ang / 2)
        p1, p2 = b + d1 * t, b + d2 * t
        cen = b + (d1 + d2).normalized() * (r / math.sin(ang / 2))
        v1, v2 = p1 - cen, p2 - cen
        phi = v1.angle(v2)
        for k in range(n + 1):
            u = k / n
            out.append(cen + (v1 * math.sin((1 - u) * phi) + v2 * math.sin(u * phi)) / math.sin(phi))
    out.append(pts[-1])
    return out


def sweep(name, pts, sections, mats, up=(0, 0, 1), caps=True, parent=None, sharp_deg=None):
    """Sweep per-point cross sections [(u, v), ...] along a path with a
    rotation minimising frame. u runs along the side vector, v along the normal."""
    pts = [Vector(p) for p in pts]
    n = len(pts)
    T = []
    for i in range(n):
        a = pts[max(i - 1, 0)]
        b = pts[min(i + 1, n - 1)]
        T.append((b - a).normalized())
    N = []
    nv = Vector(up) - Vector(up).dot(T[0]) * T[0]
    nv.normalize()
    for i in range(n):
        nv = nv - nv.dot(T[i]) * T[i]
        nv.normalize()
        N.append(nv.copy())
    V, F = [], []
    m = len(sections[0])
    for i in range(n):
        S = T[i].cross(N[i])
        for u, v in sections[i]:
            V.append(pts[i] + S * u + N[i] * v)
    for i in range(n - 1):
        for k in range(m):
            k2 = (k + 1) % m
            F.append([i * m + k, i * m + k2, (i + 1) * m + k2, (i + 1) * m + k])
    if caps:
        F.append(list(range(m - 1, -1, -1)))
        F.append([(n - 1) * m + k for k in range(m)])
    return mesh_obj(name, V, F, mats, parent=parent, sharp_deg=sharp_deg,
                    smooth=True)


def heightfield(name, xs, ys, fz, mat, parent=None):
    nx, ny = len(xs), len(ys)
    V = [(x, y, fz(x, y)) for y in ys for x in xs]
    F = [[j * nx + i, j * nx + i + 1, (j + 1) * nx + i + 1, (j + 1) * nx + i]
         for j in range(ny - 1) for i in range(nx - 1)]
    return mesh_obj(name, V, F, [mat], parent=parent, sharp_deg=None, recalc=False)


def smoothstep(e0, e1, x):
    t = min(max((x - e0) / (e1 - e0), 0.0), 1.0)
    return t * t * (3 - 2 * t)


# ---------------------------------------------------------------- copper map

PPM = 60  # texture pixels per millimetre


class CopperMap:
    """Rasterises the top copper (traces, pads, ground pour) into a texture that
    the solder mask shader reads: R = copper, G = blurred height, B = mask opening."""

    def __init__(self):
        self.nx, self.ny = int(BOARD_W * PPM), int(BOARD_H * PPM)
        z = lambda: np.zeros((self.ny, self.nx), np.float32)
        self.cu, self.clr, self.opn, self.dip = z(), z(), z(), z()

    def _win(self, x0, x1, y0, y1):
        i0 = max(int((x0 + BOARD_W / 2) * PPM) - 2, 0)
        i1 = min(int((x1 + BOARD_W / 2) * PPM) + 3, self.nx)
        j0 = max(int((y0 + BOARD_H / 2) * PPM) - 2, 0)
        j1 = min(int((y1 + BOARD_H / 2) * PPM) + 3, self.ny)
        xs = (np.arange(i0, i1) + 0.5) / PPM - BOARD_W / 2
        ys = (np.arange(j0, j1) + 0.5) / PPM - BOARD_H / 2
        X, Y = np.meshgrid(xs, ys)
        return (slice(j0, j1), slice(i0, i1)), X, Y

    @staticmethod
    def _cov(d):
        return np.clip(0.5 - d * PPM, 0, 1)

    def _put(self, arr, sl, d):
        arr[sl] = np.maximum(arr[sl], self._cov(d))

    def seg(self, p0, p1, w, clear=0.25):
        hw = w / 2
        g = hw + clear + 0.1
        sl, X, Y = self._win(min(p0[0], p1[0]) - g, max(p0[0], p1[0]) + g,
                             min(p0[1], p1[1]) - g, max(p0[1], p1[1]) + g)
        dx, dy = p1[0] - p0[0], p1[1] - p0[1]
        L2 = max(dx * dx + dy * dy, 1e-12)
        t = np.clip(((X - p0[0]) * dx + (Y - p0[1]) * dy) / L2, 0, 1)
        d = np.hypot(X - (p0[0] + t * dx), Y - (p0[1] + t * dy)) - hw
        self._put(self.cu, sl, d)
        self._put(self.clr, sl, d - clear)

    def track(self, pts, w, clear=0.25):
        for a, b in zip(pts[:-1], pts[1:]):
            self.seg(a, b, w, clear)

    def rect(self, c, size, r=0.05, clear=0.25, copper=True, opening=False):
        hx, hy = size[0] / 2, size[1] / 2
        g = max(hx, hy) + clear + 0.1
        sl, X, Y = self._win(c[0] - g, c[0] + g, c[1] - g, c[1] + g)
        qx = np.abs(X - c[0]) - (hx - r)
        qy = np.abs(Y - c[1]) - (hy - r)
        d = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r
        if copper:
            self._put(self.cu, sl, d)
        self._put(self.clr, sl, d - clear)
        if opening:
            self._put(self.opn, sl, d - 0.05)

    def circle(self, c, r, clear=0.25, copper=True, opening_r=None, dip_r=None):
        g = max(r + clear, opening_r or 0) + 0.1
        sl, X, Y = self._win(c[0] - g, c[0] + g, c[1] - g, c[1] + g)
        rr = np.hypot(X - c[0], Y - c[1])
        if copper:
            self._put(self.cu, sl, rr - r)
        self._put(self.clr, sl, rr - r - clear)
        if opening_r:
            self._put(self.opn, sl, rr - opening_r)
        if dip_r:
            self._put(self.dip, sl, rr - dip_r)

    @staticmethod
    def _blur(a, sigma):
        r = int(3 * sigma) + 1
        x = np.arange(-r, r + 1)
        k = np.exp(-x ** 2 / (2 * sigma ** 2))
        k /= k.sum()
        out = np.zeros_like(a)
        p = np.pad(a, ((0, 0), (r, r)), mode="edge")
        for i, kv in enumerate(k):
            out += kv * p[:, i:i + a.shape[1]]
        out2 = np.zeros_like(a)
        p = np.pad(out, ((r, r), (0, 0)), mode="edge")
        for i, kv in enumerate(k):
            out2 += kv * p[i:i + a.shape[0], :]
        return out2

    def image(self):
        # Ground pour: everything inside the board (0.5 mm in from the edge)
        # that is not within clearance of another feature.
        sl, X, Y = self._win(-BOARD_W, BOARD_W, -BOARD_H, BOARD_H)
        hx, hy, r = BOARD_W / 2 - 0.5, BOARD_H / 2 - 0.5, 0.6
        qx, qy = np.abs(X) - (hx - r), np.abs(Y) - (hy - r)
        d = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r
        inside = self._cov(d)
        pour = inside * (1 - self.clr)
        copper = np.maximum(self.cu, pour)
        height = self._blur(copper, 2.0) - 0.7 * self._blur(self.dip, 1.5)
        img = bpy.data.images.new("copper", self.nx, self.ny, alpha=True, float_buffer=True)
        img.colorspace_settings.name = "Non-Color"
        px = np.stack([copper, np.clip(height, 0, 1), self.opn, np.ones_like(copper)], -1)
        img.pixels.foreach_set(px.astype(np.float32).ravel())
        img.update()
        try:
            img.pack()
        except RuntimeError:
            pass
        return img


CU = CopperMap()


def to_board(parent_loc, rot_deg, x, y):
    a = math.radians(rot_deg)
    return (parent_loc[0] + x * math.cos(a) - y * math.sin(a),
            parent_loc[1] + x * math.sin(a) + y * math.cos(a))


def pad_to_board(loc, rot, c, size):
    bx, by = to_board(loc, rot, c[0], c[1])
    sx, sy = size if int(round(rot / 90)) % 2 == 0 else (size[1], size[0])
    return (bx, by), (sx, sy)


# ---------------------------------------------------------------- materials

def new_mat(name):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    return m, m.node_tree.nodes, m.node_tree.links


def bsdf_of(m):
    return next(n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED")


def principled(name, base, rough, metal=0.0, spec=0.5, coat=0.0, coat_rough=0.05,
               aniso=0.0, sss=0.0):
    m, N, L = new_mat(name)
    b = bsdf_of(m)
    b.inputs["Base Color"].default_value = (*base, 1)
    b.inputs["Roughness"].default_value = rough
    b.inputs["Metallic"].default_value = metal
    b.inputs["Specular IOR Level"].default_value = spec
    b.inputs["Coat Weight"].default_value = coat
    b.inputs["Coat Roughness"].default_value = coat_rough
    b.inputs["Anisotropic"].default_value = aniso
    if sss:
        b.inputs["Subsurface Weight"].default_value = sss
        b.inputs["Subsurface Radius"].default_value = (0.05, 0.04, 0.03)
    return m


def mix_rgb(N, L, fac, a, b):
    mx = N.new("ShaderNodeMix")
    mx.data_type = "RGBA"
    for sock, val in ((mx.inputs[0], fac), (mx.inputs[6], a), (mx.inputs[7], b)):
        if isinstance(val, bpy.types.NodeSocket):
            L.new(val, sock)
        elif isinstance(val, (int, float)):
            sock.default_value = val
        else:
            sock.default_value = (*val, 1) if len(val) == 3 else val
    return mx.outputs[2]


def math_node(N, L, op, a, b=None, c=None):
    n = N.new("ShaderNodeMath")
    n.operation = op
    for sock, val in zip(n.inputs, (a, b, c)):
        if val is None:
            continue
        if isinstance(val, bpy.types.NodeSocket):
            L.new(val, sock)
        else:
            sock.default_value = val
    return n.outputs[0]


def noise(N, L, vec, scale, detail=3.0, rough=0.55):
    t = N.new("ShaderNodeTexNoise")
    t.inputs["Scale"].default_value = scale
    t.inputs["Detail"].default_value = detail
    t.inputs["Roughness"].default_value = rough
    if vec is not None:
        L.new(vec, t.inputs["Vector"])
    return t.outputs["Fac"]


def mat_mask(img):
    m, N, L = new_mat("solder_mask")
    b = bsdf_of(m)
    tc = N.new("ShaderNodeTexCoord")
    vm = N.new("ShaderNodeVectorMath")
    vm.operation = "MULTIPLY_ADD"
    vm.inputs[1].default_value = (1 / BOARD_W, 1 / BOARD_H, 0)
    vm.inputs[2].default_value = (0.5, 0.5, 0)
    L.new(tc.outputs["Object"], vm.inputs[0])
    tex = N.new("ShaderNodeTexImage")
    tex.image = img
    tex.interpolation = "Cubic"
    tex.extension = "EXTEND"
    L.new(vm.outputs[0], tex.inputs["Vector"])
    sep = N.new("ShaderNodeSeparateColor")
    L.new(tex.outputs["Color"], sep.inputs["Color"])
    cu, hgt, opn = sep.outputs["Red"], sep.outputs["Green"], sep.outputs["Blue"]
    grain = noise(N, L, tc.outputs["Object"], 1.2, 3.0, 0.5)
    bare = hex_lin("#0b1615")
    over = hex_lin("#142a27")
    col = mix_rgb(N, L, cu, bare, over)
    col = mix_rgb(N, L, math_node(N, L, "MULTIPLY", grain, 0.08), col, hex_lin("#122321"))
    col = mix_rgb(N, L, opn, col, hex_lin("#3a3626"))
    L.new(col, b.inputs["Base Color"])
    rough = math_node(N, L, "MULTIPLY_ADD", cu, -0.06, 0.54)
    rough = math_node(N, L, "MULTIPLY_ADD", grain, 0.08, rough)
    L.new(rough, b.inputs["Roughness"])
    b.inputs["Specular IOR Level"].default_value = 0.5
    fine = noise(N, L, tc.outputs["Object"], 90.0, 2.0, 0.5)
    h = math_node(N, L, "MULTIPLY_ADD", fine, 0.05, hgt)
    bump = N.new("ShaderNodeBump")
    bump.inputs["Distance"].default_value = 0.03
    bump.inputs["Strength"].default_value = 1.0
    L.new(h, bump.inputs["Height"])
    L.new(bump.outputs["Normal"], b.inputs["Normal"])
    return m


def mat_fr4_edge():
    """Routed board edge: glass weave laminate with thin dark mask lines."""
    m, N, L = new_mat("fr4_edge")
    b = bsdf_of(m)
    tc = N.new("ShaderNodeTexCoord")
    sepz = N.new("ShaderNodeSeparateXYZ")
    L.new(tc.outputs["Object"], sepz.inputs[0])
    z = sepz.outputs["Z"]
    mp = N.new("ShaderNodeMapping")
    mp.inputs["Scale"].default_value = (1.5, 1.5, 28.0)
    L.new(tc.outputs["Object"], mp.inputs["Vector"])
    weave = noise(N, L, mp.outputs["Vector"], 3.0, 2.0, 0.5)
    lam = mix_rgb(N, L, weave, hex_lin("#77766a"), hex_lin("#939181"))
    top = math_node(N, L, "GREATER_THAN", z, BOARD_T - 0.05)
    bot = math_node(N, L, "LESS_THAN", z, 0.05)
    band = math_node(N, L, "MAXIMUM", top, bot)
    col = mix_rgb(N, L, band, lam, hex_lin("#0f1a18"))
    L.new(col, b.inputs["Base Color"])
    b.inputs["Roughness"].default_value = 0.78
    bump = N.new("ShaderNodeBump")
    bump.inputs["Distance"].default_value = 0.01
    L.new(weave, bump.inputs["Height"])
    L.new(bump.outputs["Normal"], b.inputs["Normal"])
    return m


def mat_mold():
    """Epoxy mould compound with a fine stippled finish."""
    m = principled("mold", hex_lin("#121313"), 0.55, spec=0.5)
    N, L = m.node_tree.nodes, m.node_tree.links
    b = bsdf_of(m)
    tc = N.new("ShaderNodeTexCoord")
    f = noise(N, L, tc.outputs["Object"], 60.0, 3.0, 0.7)
    bump = N.new("ShaderNodeBump")
    bump.inputs["Distance"].default_value = 0.004
    L.new(f, bump.inputs["Height"])
    L.new(bump.outputs["Normal"], b.inputs["Normal"])
    L.new(math_node(N, L, "MULTIPLY_ADD", f, 0.12, 0.5), b.inputs["Roughness"])
    return m


def mat_aluminium():
    m = principled("aluminium", (0.86, 0.87, 0.88), 0.32, metal=1.0, aniso=0.5)
    N, L = m.node_tree.nodes, m.node_tree.links
    b = bsdf_of(m)
    tg = N.new("ShaderNodeTangent")
    tg.direction_type = "RADIAL"
    tg.axis = "Z"
    L.new(tg.outputs["Tangent"], b.inputs["Tangent"])
    # Pressure vent: two shallow crossed grooves scored into the top.
    tc = N.new("ShaderNodeTexCoord")
    sp = N.new("ShaderNodeSeparateXYZ")
    L.new(tc.outputs["Object"], sp.inputs[0])
    ax = math_node(N, L, "ABSOLUTE", sp.outputs["X"])
    ay = math_node(N, L, "ABSOLUTE", sp.outputs["Y"])
    groove = math_node(N, L, "LESS_THAN", math_node(N, L, "MINIMUM", ax, ay), 0.05)
    rr = math_node(N, L, "LESS_THAN", math_node(N, L, "ADD", math_node(N, L, "MULTIPLY", ax, ax),
                                                math_node(N, L, "MULTIPLY", ay, ay)), 1.9 * 1.9)
    on_top = math_node(N, L, "GREATER_THAN", sp.outputs["Z"], 5.3)
    g = math_node(N, L, "MULTIPLY", math_node(N, L, "MULTIPLY", groove, rr), on_top)
    bump = N.new("ShaderNodeBump")
    bump.inputs["Distance"].default_value = 0.03
    bump.invert = True
    L.new(g, bump.inputs["Height"])
    L.new(bump.outputs["Normal"], b.inputs["Normal"])
    return m


def mat_glow(strength):
    """Luminous light-pipe: teal emission, brighter along the core, with a
    clear glossy skin so it still reads as an object on a light background."""
    m, N, L = new_mat("glow")
    b = bsdf_of(m)
    b.inputs["Base Color"].default_value = (*hex_lin("#0d7a68"), 1)
    b.inputs["Roughness"].default_value = 0.25
    b.inputs["Coat Weight"].default_value = 1.0
    b.inputs["Coat Roughness"].default_value = 0.08
    lw = N.new("ShaderNodeLayerWeight")
    lw.inputs["Blend"].default_value = 0.35
    facing = lw.outputs["Facing"]
    core = math_node(N, L, "POWER", math_node(N, L, "SUBTRACT", 1.0, facing), 2.0)
    col = mix_rgb(N, L, math_node(N, L, "MULTIPLY", core, 0.55), hex_lin("#19c2a4"),
                  hex_lin("#b8fff0"))
    L.new(col, b.inputs["Emission Color"])
    L.new(math_node(N, L, "MULTIPLY_ADD", core, strength * 0.6, strength * 0.4),
          b.inputs["Emission Strength"])
    return m


def mat_floor():
    if DARK:
        return principled("floor", (0.0074, 0.0050, 0.0047), 0.5, spec=0.3)
    return principled("floor", (0.80, 0.80, 0.79), 0.92, spec=0.2)


MAT = {}


def build_materials(copper_img):
    MAT["mask"] = mat_mask(copper_img)
    MAT["fr4"] = mat_fr4_edge()
    MAT["gold"] = principled("enig_gold", (0.94, 0.72, 0.40), 0.27, metal=1.0)
    MAT["solder"] = principled("solder", (0.72, 0.73, 0.74), 0.1, metal=1.0)
    MAT["tin"] = principled("tin", (0.62, 0.62, 0.61), 0.27, metal=1.0)
    MAT["mold"] = mat_mold()
    MAT["dimple"] = principled("mold_dimple", hex_lin("#141515"), 0.28)
    MAT["alumina"] = principled("alumina", (0.72, 0.71, 0.66), 0.65, sss=0.2)
    MAT["overcoat"] = principled("res_overcoat", hex_lin("#0c0d0d"), 0.32)
    MAT["mlcc"] = principled("mlcc", hex_lin("#a08466"), 0.5, sss=0.15)
    MAT["silk"] = principled("silkscreen", (0.80, 0.80, 0.78), 0.62)
    MAT["plastic"] = principled("black_plastic", hex_lin("#101111"), 0.45)
    MAT["alu"] = mat_aluminium()
    MAT["print"] = principled("cap_print", hex_lin("#0b0c0c"), 0.35)
    MAT["hole"] = principled("hole", (0.01, 0.01, 0.01), 0.6)
    MAT["glow"] = mat_glow(9.0 if DARK else 1.6)
    MAT["floor"] = mat_floor()


# ---------------------------------------------------------------- board

def board_outline():
    """Rounded rectangle with castellated half-holes on the left and right
    edges. Returns the CCW polygon and a per-edge flag (True = plated)."""
    r, seg, nh = 1.0, 8, 14
    hw, hh = BOARD_W / 2, BOARD_H / 2
    pts, plated = [], []

    def arc(cx, cy, a0, a1, n, rad, flag):
        for k in range(n + 1):
            a = math.radians(a0 + (a1 - a0) * k / n)
            pts.append((cx + rad * math.cos(a), cy + rad * math.sin(a)))
            plated.append(flag and k < n)

    arc(hw - r, -hh + r, -90, 0, seg, r, False)          # bottom right corner
    for y in CAST_Y:                                      # right edge, going up
        arc(hw, y, -90, -270, nh, CAST_R, True)
    arc(hw - r, hh - r, 0, 90, seg, r, False)            # top right
    arc(-hw + r, hh - r, 90, 180, seg, r, False)         # top left
    for y in reversed(CAST_Y):                            # left edge, going down
        arc(-hw, y, 90, -90, nh, CAST_R, True)
    arc(-hw + r, -hh + r, 180, 270, seg, r, False)       # bottom left
    return pts, plated


def build_board():
    poly, plated = board_outline()
    side = [2 if p else 1 for p in plated]
    V, F, M = prism(poly, 0.0, BOARD_T, side)
    ob = mesh_obj("board", V, F, [MAT["mask"], MAT["fr4"], MAT["gold"]], mat_idx=M,
                  sharp_deg=45)
    return ob


def castellation_pad(x_edge, y, side):
    """Top-side pad of a castellated hole. side=+1 for the right edge."""
    L, w, nh = 1.4, 1.0, 14
    pts = []
    xin = x_edge - side * (L - w / 2)
    # inner rounded end, then out to the edge, around the notch, and back
    if side > 0:
        for k in range(nh + 1):
            a = math.radians(90 + 180 * k / nh)
            pts.append((xin + w / 2 * math.cos(a), y + w / 2 * math.sin(a)))
        pts.append((x_edge, y - w / 2))
        for k in range(nh + 1):
            a = math.radians(-90 - 180 * k / nh)
            pts.append((x_edge + CAST_R * math.cos(a), y + CAST_R * math.sin(a)))
        pts.append((x_edge, y + w / 2))
    else:
        for k in range(nh + 1):
            a = math.radians(-90 + 180 * k / nh)
            pts.append((xin + w / 2 * math.cos(a), y + w / 2 * math.sin(a)))
        pts.append((x_edge, y + w / 2))
        for k in range(nh + 1):
            a = math.radians(90 - 180 * k / nh)
            pts.append((x_edge + CAST_R * math.cos(a), y + CAST_R * math.sin(a)))
        pts.append((x_edge, y - w / 2))
    V, F, M = prism(pts, ZT - 0.005, ZT + PAD_H)
    mesh_obj("cast_pad", V, F, [MAT["gold"]], sharp_deg=40)
    CU.rect((x_edge - side * L / 2, y), (L + 0.1, w), r=0.3)


def gold_round(name, c, r, opening_r=None):
    cylinder(name, r, ZT - 0.005, ZT + PAD_H, MAT["gold"], c[0], c[1], n=40)
    CU.circle(c, r, opening_r=opening_r)


def via(c, tented=False):
    if tented:
        CU.circle(c, 0.3, clear=0.2, dip_r=0.13)
        return
    lathe("via", [(0.0, ZT - 0.5), (0.15, ZT - 0.5), (0.15, ZT + PAD_H), (0.3, ZT + PAD_H),
                  (0.3, ZT - 0.005)], [MAT["hole"], MAT["gold"]], seg_mat=[0, 1, 1, 1],
          n=32, cx=c[0], cy=c[1])
    CU.circle(c, 0.3, clear=0.2)


# ---------------------------------------------------------------- solder

def chip_solder(name, parent, xe, pad_c, pad_w, pad_h, hf):
    """Reflowed fillet on the right pad (mirrored when pad_c < 0)."""
    sgn = 1 if pad_c > 0 else -1
    px0, px1 = abs(pad_c) - pad_w / 2, abs(pad_c) + pad_w / 2
    hy = pad_h / 2
    xs = sorted(set(np.round(np.concatenate([
        np.linspace(px0, xe, 6), np.linspace(xe, xe + 0.06, 4),
        np.linspace(xe + 0.06, px1, 14)]), 5)))
    ys = list(np.linspace(-hy, hy, 15))

    def fz(x, y):
        dx = min(x - px0, px1 - x)
        dy = hy - abs(y)
        e = smoothstep(0.0, 0.08, min(dx, dy))
        f = hf if x < xe else hf * (max(px1 - x, 0.0) / (px1 - xe)) ** 1.7
        gy = max(1 - (abs(y) / hy) ** 4, 0)
        return -0.004 + (PAD_H + 0.012 + f * gy) * e

    ob = heightfield(name, [sgn * x for x in xs], ys, lambda x, y: fz(sgn * x, y),
                     MAT["solder"], parent)
    if sgn < 0:  # mirrored grid flips the winding
        ob.data.flip_normals()
    return ob


def gull_solder(name, parent, x, py0, py1, pw, heel, toe, lw, sgn, h_heel=0.26):
    """Fillet around a gull-wing foot. The pad spans |y| in [py0, py1]; the
    foot runs from |y| = heel to toe. sgn picks the side of the package."""
    xs = list(np.linspace(x - pw / 2, x + pw / 2, 13))
    us = list(np.linspace(py0, py1, 22))

    def fz(xx, u):
        dx = pw / 2 - abs(xx - x)
        dy = min(u - py0, py1 - u)
        e = smoothstep(0.0, 0.07, min(dx, dy))
        gx = max(1 - (abs(xx - x) / (lw / 2 + 0.1)) ** 2, 0)
        hh = h_heel * math.exp(-((u - heel) / 0.16) ** 2)
        if heel <= u <= toe:
            along = 0.08
        else:
            dd = heel - u if u < heel else u - toe
            along = 0.08 * math.exp(-(dd / 0.14) ** 2)
        return -0.004 + (PAD_H + 0.01 + (hh + along) * gx) * e

    ob = heightfield(name, xs, [sgn * u for u in us], lambda xx, yy: fz(xx, sgn * yy),
                     MAT["solder"], parent)
    if sgn < 0:
        ob.data.flip_normals()
    return ob


def gull_lead(name, parent, x, y_in, y_knee, y_foot, y_end, z_exit, lw, lt, sgn):
    zf = PAD_H + 0.03 + lt / 2
    path = fillet_path([(x, sgn * y_in, z_exit), (x, sgn * y_knee, z_exit),
                        (x, sgn * y_foot, zf), (x, sgn * y_end, zf)], 0.12, 8)
    sec = [(-lw / 2, -lt / 2), (lw / 2, -lt / 2), (lw / 2, lt / 2), (-lw / 2, lt / 2)]
    ob = sweep(name, path, [sec] * len(path), [MAT["tin"]], up=(0, 0, 1), sharp_deg=50,
               parent=parent)
    add_bevel(ob, 0.025, 2, 60)
    return ob


# ---------------------------------------------------------------- components

def chip0603(name, loc, rot, kind="R", label=None, label_off=(0, 1.15), label_rot=0):
    e = empty(name, (loc[0], loc[1], ZT), rot)
    L, W = 1.6, 0.8
    H = 0.45 if kind == "R" else 0.8
    z0 = PAD_H + 0.03
    cap = 0.3 if kind == "R" else 0.35
    if kind == "R":
        box(name + "_core", (L - 0.02, W - 0.02, H - 0.06), (0, 0, z0 + (H - 0.06) / 2),
            MAT["alumina"], e, bevel=0.02)
        box(name + "_coat", (L - 2 * cap + 0.1, W - 0.08, 0.06), (0, 0, z0 + H - 0.05),
            MAT["overcoat"], e, bevel=0.025)
    else:
        box(name + "_body", (L - 0.02, W - 0.02, H - 0.02), (0, 0, z0 + H / 2),
            MAT["mlcc"], e, bevel=0.06)
    for s in (-1, 1):
        box(name + "_term", (cap, W + 0.01, H + 0.01), (s * (L / 2 - cap / 2 + 0.005), 0, z0 + H / 2),
            MAT["tin"], e, bevel=0.06)
    pw, ph, pc = 0.85, 0.95, 0.8
    for s in (-1, 1):
        chip_solder(name + "_sj", e, L / 2, s * pc, pw, ph, 0.26 if kind == "R" else 0.36)
        c, sz = pad_to_board(loc, rot, (s * pc, 0), (pw, ph))
        CU.rect(c, sz)
    if label:
        silk_text(label, *to_board(loc, rot, *label_off), h=0.75, rot=label_rot)


def pads0603(name, loc, rot, label=None, label_off=(0, 1.15)):
    """Unpopulated footprint: bare ENIG pads."""
    for s in (-1, 1):
        c, sz = pad_to_board(loc, rot, (s * 0.8, 0), (0.85, 0.95))
        V, F, M = prism(rrect(sz[0], sz[1], 0.08, 4, c[0], c[1]), ZT - 0.005, ZT + PAD_H)
        mesh_obj(name + "_pad", V, F, [MAT["gold"]], sharp_deg=40)
        CU.rect(c, sz)
    if label:
        silk_text(label, *to_board(loc, rot, *label_off), h=0.75)


def soic8(name, loc, rot, label_off=(0, 4.1)):
    e = empty(name, (loc[0], loc[1], ZT), rot)
    zb = PAD_H + 0.12
    ring_stack(name + "_body", [(4.75, 3.72, zb), (4.9, 3.9, zb + 0.58), (4.74, 3.74, zb + 1.42)],
               MAT["mold"], e, bevel=0.07)
    cylinder(name + "_dimple", 0.32, zb + 1.415, zb + 1.424, MAT["dimple"], -1.62, -1.02,
             n=40, parent=e)
    for i, px in enumerate((-1.905, -0.635, 0.635, 1.905)):
        for sgn in (-1, 1):
            gull_lead(name + "_lead", e, px, 1.75, 2.2, 2.5, 3.0, zb + 0.58, 0.42, 0.2, sgn)
            gull_solder(name + "_sj", e, px, 1.95, 3.45, 0.62, 2.5, 3.0, 0.42, sgn)
            c, sz = pad_to_board(loc, rot, (px, sgn * 2.7), (0.62, 1.5))
            CU.rect(c, sz)
    # silkscreen: pin 1 dot
    silk_dot(to_board(loc, rot, -2.9, -3.15), 0.16)
    silk_text("U1", *to_board(loc, rot, *label_off), h=0.85)


def sot23(name, loc, rot, label=None, label_off=(0, 2.1)):
    e = empty(name, (loc[0], loc[1], ZT), rot)
    zb = PAD_H + 0.08
    ring_stack(name + "_body", [(2.82, 1.24, zb), (2.92, 1.32, zb + 0.4), (2.84, 1.25, zb + 0.95)],
               MAT["mold"], e, bevel=0.05)
    for px, sgn in ((-0.95, -1), (0.95, -1), (0.0, 1)):
        gull_lead(name + "_lead", e, px, 0.5, 0.75, 0.92, 1.25, zb + 0.4, 0.4, 0.13, sgn)
        gull_solder(name + "_sj", e, px, 0.62, 1.62, 0.62, 0.92, 1.25, 0.4, sgn, 0.18)
        c, sz = pad_to_board(loc, rot, (px, sgn * 1.12), (0.62, 1.0))
        CU.rect(c, sz)
    if label:
        silk_text(label, *to_board(loc, rot, *label_off), h=0.75)


def electrolytic(name, loc):
    """SMD aluminium electrolytic, 5 mm can on a black plastic seat."""
    e = empty(name, (loc[0], loc[1], ZT), 0)
    s, ch = 5.3, 0.9
    seat = [(-s / 2 + ch, -s / 2), (s / 2, -s / 2), (s / 2, s / 2), (-s / 2 + ch, s / 2),
            (-s / 2, s / 2 - ch), (-s / 2, -s / 2 + ch)]
    V, F, M = prism(seat, 0.05, 0.55)
    ob = mesh_obj(name + "_seat", V, F, [MAT["plastic"]], parent=e, sharp_deg=30)
    add_bevel(ob, 0.05, 2, 30)
    prof = [(0.0, 0.55), (2.38, 0.55), (2.5, 0.65), (2.5, 1.05), (2.43, 1.13), (2.43, 1.27),
            (2.5, 1.35), (2.5, 5.18)]
    for k in range(1, 7):
        a = math.radians(90 * k / 6)
        prof.append((2.3 + 0.2 * math.cos(a), 5.18 + 0.2 * math.sin(a)))
    prof += [(2.0, 5.38), (0.0, 5.38)]
    lathe(name + "_can", prof, [MAT["alu"]], n=72, parent=e, sharp=60)
    # polarity half-moon printed on the negative side of the top
    pts = [(0.0, 0.0)] + [(2.05 * math.cos(math.radians(a)), 2.05 * math.sin(math.radians(a)))
                          for a in np.linspace(-60, 60, 25)]
    V, F, M = prism(pts, 5.375, 5.385)
    mesh_obj(name + "_mark", V, F, [MAT["print"]], parent=e, sharp_deg=30)
    for sx in (-1, 1):
        box(name + "_tab", (0.7, 0.6, 0.1), (sx * 2.8, 0, PAD_H + 0.05), MAT["tin"], e,
            bevel=0.02)
        chip_solder(name + "_sj", e, 3.15, sx * 2.9, 1.4, 1.6, 0.12)
        CU.rect((loc[0] + sx * 2.9, loc[1]), (1.4, 1.6))
    # silkscreen outline with a chamfer on the + side and a plus sign
    o = 3.0
    silk_line([(loc[0] + o, loc[1] - o + 0.0), (loc[0] - o + 1.0, loc[1] - o),
               (loc[0] - o, loc[1] - o + 1.0), (loc[0] - o, loc[1] - 1.0)])
    silk_line([(loc[0] + o, loc[1] + o), (loc[0] - o + 1.0, loc[1] + o),
               (loc[0] - o, loc[1] + o - 1.0), (loc[0] - o, loc[1] + 1.0)])
    silk_text("+", loc[0] - 3.9, loc[1] - 2.6, h=0.8)


# ---------------------------------------------------------------- silkscreen

SILK_W = 0.13


def _arc(cx, cy, rx, ry, a0, a1, n=10):
    return [(cx + rx * math.cos(math.radians(a0 + (a1 - a0) * k / n)),
             cy + ry * math.sin(math.radians(a0 + (a1 - a0) * k / n))) for k in range(n + 1)]


# Single-stroke glyphs on a cap height of 1, like a CAD plotter font.
GLYPHS = {
    "1": [[(0.12, 0.78), (0.36, 1.0), (0.36, 0.0)]],
    "2": [_arc(0.3, 0.7, 0.3, 0.3, 160, -30) + [(0.0, 0.0), (0.62, 0.0)]],
    "3": [_arc(0.3, 0.76, 0.28, 0.24, 150, -90) + _arc(0.3, 0.26, 0.31, 0.26, 90, -150)],
    "4": [[(0.48, 0.0), (0.48, 1.0), (0.0, 0.3), (0.66, 0.3)]],
    "U": [[(0.0, 1.0), (0.0, 0.32)] + _arc(0.3, 0.32, 0.3, 0.32, 180, 360) + [(0.6, 1.0)]],
    "R": [[(0.0, 0.0), (0.0, 1.0), (0.36, 1.0)] + _arc(0.36, 0.75, 0.25, 0.25, 90, -90)
          + [(0.0, 0.5)], [(0.3, 0.5), (0.62, 0.0)]],
    "C": [_arc(0.34, 0.5, 0.34, 0.5, 48, 312, 16)],
    "Q": [_arc(0.33, 0.5, 0.33, 0.5, 0, 360, 24), [(0.4, 0.22), (0.68, -0.05)]],
    "T": [[(0.0, 1.0), (0.64, 1.0)], [(0.32, 1.0), (0.32, 0.0)]],
    "P": [[(0.0, 0.0), (0.0, 1.0), (0.36, 1.0)] + _arc(0.36, 0.75, 0.25, 0.25, 90, -90)
          + [(0.0, 0.5)]],
    "+": [[(0.3, 0.2), (0.3, 0.8)], [(0.0, 0.5), (0.6, 0.5)]],
}
ADVANCE = 0.86
SILK = {"V": [], "F": []}


def _capsule(p0, p1, w, z0, z1, n=10):
    V, F = SILK["V"], SILK["F"]
    (x0, y0), (x1, y1) = p0, p1
    dx, dy = x1 - x0, y1 - y0
    L = math.hypot(dx, dy)
    if L > 1e-6:
        nx, ny = -dy / L * w / 2, dx / L * w / 2
        quad = [(x0 + nx, y0 + ny), (x1 + nx, y1 + ny), (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)]
        b = len(V)
        for z in (z0, z1):
            V.extend((x, y, z) for x, y in quad)
        F.extend([[b, b + 3, b + 2, b + 1], [b + 4, b + 5, b + 6, b + 7],
                  [b, b + 1, b + 5, b + 4], [b + 1, b + 2, b + 6, b + 5],
                  [b + 2, b + 3, b + 7, b + 6], [b + 3, b, b + 4, b + 7]])
    for (cx, cy) in (p0, p1):
        b = len(V)
        ring = circle_pts(w / 2, n, cx, cy)
        for z in (z0, z1):
            V.extend((x, y, z) for x, y in ring)
        F.append(list(range(b + n - 1, b - 1, -1)))
        F.append(list(range(b + n, b + 2 * n)))
        for k in range(n):
            k2 = (k + 1) % n
            F.append([b + k, b + k2, b + n + k2, b + n + k])


def silk_line(pts, w=SILK_W):
    for a, b in zip(pts[:-1], pts[1:]):
        _capsule(a, b, w, ZT - 0.004, ZT + 0.016)


def silk_dot(c, r):
    _capsule(c, c, 2 * r, ZT - 0.004, ZT + 0.016, n=16)


def silk_text(s, x, y, h=0.8, rot=0.0):
    """Centred single-stroke text at board position (x, y)."""
    width = (len(s) - 1) * ADVANCE * h + 0.62 * h
    a = math.radians(rot)
    ca, sa = math.cos(a), math.sin(a)
    for i, ch in enumerate(s):
        ox = -width / 2 + i * ADVANCE * h
        for stroke in GLYPHS[ch]:
            pts = []
            for gx, gy in stroke:
                lx, ly = ox + gx * h, (gy - 0.5) * h
                pts.append((x + lx * ca - ly * sa, y + lx * sa + ly * ca))
            silk_line(pts, w=SILK_W * h / 0.8)


def build_silk():
    mesh_obj("silkscreen", SILK["V"], SILK["F"], [MAT["silk"]], smooth=False, recalc=False)


# ---------------------------------------------------------------- the signal

def bezier(p0, p1, p2, p3, n):
    out = []
    for k in range(n + 1):
        t = k / n
        u = 1 - t
        out.append((u ** 3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t ** 3 * p3[0],
                    u ** 3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t ** 3 * p3[1]))
    return out


def build_signal():
    """Glowing trace from a via (the logo dot) to the castellated pad, then a
    luminous tube that leaves the board edge along the logo waveform."""
    x_rise = 10.2                         # logo x where the rise starts = board edge
    x_dot = XE - (x_rise - 6.0) * WAVE_S  # logo dot
    z_air = ZT + PAD_H + TUBE_R           # tube centre where it leaves the pad
    flat_a, flat_b, flat_c = 0.19, 0.03, 0.012
    m0, m1 = XE - 1.9, XE - 0.25          # flat ribbon morphs into the round tube

    pts, secs = [], []

    def section(a, b, n=20):
        return [(a * math.cos(2 * math.pi * k / n), b * math.sin(2 * math.pi * k / n))
                for k in range(n)]

    for x in np.arange(x_dot, XE, 0.05):
        t = smoothstep(m0, m1, x)
        a = flat_a + (TUBE_R - flat_a) * t
        b = flat_b + (TUBE_R - flat_b) * t
        zc = ZT + flat_c + (PAD_H + TUBE_R - flat_c) * t
        pts.append((x, SIG_Y, zc))
        secs.append(section(a, b))
    for seg in WAVE[1:]:
        for i, (lx, ly) in enumerate(bezier(*seg, 140)):
            if i == 0 and pts and seg is not WAVE[1]:
                continue
            pts.append((XE + (lx - x_rise) * WAVE_S, SIG_Y, z_air + (21.0 - ly) * WAVE_S))
            secs.append(section(TUBE_R, TUBE_R))
    sweep("signal", pts, secs, [MAT["glow"]], up=(0, 0, 1))
    end = pts[-1]
    bpy.ops.mesh.primitive_uv_sphere_add(radius=TUBE_R, segments=24, ring_count=12, location=end)
    cap = bpy.context.active_object
    cap.data.materials.append(MAT["glow"])
    cap.data.polygons.foreach_set("use_smooth", [True] * len(cap.data.polygons))
    # the logo dot: a domed glowing via at the start of the trace
    bpy.ops.mesh.primitive_uv_sphere_add(radius=1.0, segments=32, ring_count=16,
                                         location=(x_dot, SIG_Y, ZT + 0.0))
    dot = bpy.context.active_object
    dot.scale = (0.4, 0.4, 0.08)
    dot.data.materials.append(MAT["glow"])
    dot.data.polygons.foreach_set("use_smooth", [True] * len(dot.data.polygons))
    for ob in (cap, dot, bpy.data.objects["signal"]):
        ob.visible_shadow = False
    CU.track([(x_dot, SIG_Y), (XE - 1.0, SIG_Y)], 0.3)
    CU.circle((x_dot, SIG_Y), 0.42)
    return x_dot


# ---------------------------------------------------------------- layout

def build_layout():
    for y in CAST_Y:
        castellation_pad(XE, y, 1)
        castellation_pad(-XE, y, -1)

    # IC and the parts around the signal path
    soic8("U1", (9.6, 0.65), 0)
    chip0603("R1", (13.9, -0.15), 90, "R", "R1", label_off=(0.0, 1.2))
    chip0603("C1", (13.9, 3.45), 90, "C", "C1", label_off=(0.0, 1.2))
    sot23("Q1", (10.2, 7.15), 0, "Q1", label_off=(-2.4, 0.0))
    chip0603("C2", (6.2, 6.4), 0, "C", "C2")
    chip0603("R2", (13.7, 7.7), 0, "R", "R2", label_off=(0, -1.2))
    pads0603("R3", (7.4, -6.9), 0, "R3")
    gold_round("TP1", (13.5, -7.0), 0.6)
    silk_text("TP1", 13.5, -8.35, h=0.7)
    gold_round("fid", (11.0, -9.4), 0.5, opening_r=1.0)
    gold_round("fid", (-14.6, 9.4), 0.5, opening_r=1.0)

    # left half, mostly out of focus
    electrolytic("C4", (-8.0, 3.6))
    silk_text("C4", -8.0, 7.6, h=0.85)
    chip0603("R4", (-1.6, -2.6), 90, "R", "R4", label_off=(0, -1.2))
    chip0603("C3", (0.4, -2.6), 90, "C", "C3", label_off=(0, -1.2))
    chip0603("R5", (-2.0, 6.8), 0, "R")
    for c in [(-11.5, -6.0), (-9.0, -6.0)]:
        via(c)

    # vias
    for c in [(4.0, -4.6), (5.6, 4.4), (-4.2, 1.2), (12.2, -5.2), (11.9, 4.6), (2.6, 8.6)]:
        via(c)
    for i in range(12):
        via((-13.0 + 2.2 * i, -10.0), tented=True)
    for i in range(6):
        via((-13.0 + 2.2 * i, 10.0), tented=True)

    # copper routed under the mask (seen as faint relief)
    CU.track([(13.9, -0.95), (14.27, -1.27), (15.7, -1.27)], 0.3)
    CU.track([(13.9, 4.25), (14.36, 3.81), (15.7, 3.81)], 0.3)
    CU.track([(13.9, 0.65), (13.9, 1.6), (12.8, 2.7), (12.8, 3.6)], 0.25)
    CU.track([(11.505, 3.35), (11.505, 4.6), (11.9, 4.6)], 0.25)
    CU.track([(10.2, 6.0), (10.2, 5.2), (9.6, 4.6), (8.965, 4.6), (8.965, 3.35)], 0.25)
    CU.track([(10.235, -1.35), (10.235, -2.6), (11.0, -3.0)], 0.25)
    CU.track([(7.695, -1.35), (7.695, -3.2), (6.4, -4.5), (4.0, -4.6)], 0.25)
    CU.track([(8.965, -1.35), (8.965, -2.2)], 0.25)
    CU.track([(12.65, 7.7), (14.6, 7.7), (15.7, 6.35)], 0.25)
    CU.track([(6.6, -6.9), (5.2, -6.9), (4.2, -5.9), (4.0, -4.6)], 0.25)
    CU.track([(8.2, -6.9), (10.4, -6.9), (12.2, -5.2)], 0.25)
    CU.track([(5.4, 6.4), (5.6, 4.4)], 0.25)
    CU.track([(-15.7, 1.27), (-12.5, 1.27), (-10.8, 3.0), (-10.6, 3.6)], 0.6)
    CU.track([(-5.4, 3.6), (-3.2, 3.6), (-1.6, 2.0), (-1.6, -1.8)], 0.6)
    CU.track([(-15.7, -3.81), (-6.0, -3.81), (-4.2, -2.0), (-4.2, 1.2)], 0.25)
    CU.track([(0.4, -3.4), (0.4, -5.2), (2.0, -6.8), (6.6, -6.8)], 0.25)
    CU.track([(0.4, -1.8), (0.4, 0.4), (2.2, 2.2), (7.695, 2.2), (7.695, 3.35)], 0.25)
    CU.track([(-2.8, 6.8), (-4.0, 6.8), (-4.0, 9.0), (-15.7, 8.89)], 0.25)
    CU.track([(-1.2, 6.8), (2.6, 6.8), (2.6, 8.6)], 0.25)
    CU.track([(-15.7, -6.35), (-12.6, -6.35), (-11.5, -6.0)], 0.25)
    CU.track([(-9.0, -6.0), (-6.0, -6.0), (-4.0, -8.0), (6.0, -8.0)], 0.25)


# ---------------------------------------------------------------- lights, camera, world

def area_light(name, size, size_y, pos, target, energy, color=(1, 1, 1), spread=180):
    ld = bpy.data.lights.new(name, "AREA")
    ld.shape = "RECTANGLE"
    ld.size, ld.size_y = size, size_y
    ld.energy = energy
    ld.normalize = False
    ld.color = color
    ld.spread = math.radians(spread)
    ob = bpy.data.objects.new(name, ld)
    ob.location = pos
    ob.rotation_euler = (Vector(target) - Vector(pos)).to_track_quat("-Z", "Y").to_euler()
    ob.visible_camera = False
    COLL.objects.link(ob)
    return ob


def sph(az, el, dist, target):
    az, el = math.radians(az), math.radians(el)
    return Vector(target) + dist * Vector((math.cos(el) * math.cos(az),
                                           math.cos(el) * math.sin(az), math.sin(el)))


def build_lights():
    tgt = Vector((10.0, -1.0, 2.0))
    if DARK:
        area_light("key", 220, 160, sph(205, 48, 330, tgt), tgt, 2.6, (1.0, 0.98, 0.95))
        area_light("rim", 300, 70, sph(120, 38, 300, tgt), tgt, 6.0, (0.92, 0.97, 1.0))
        area_light("kicker", 120, 260, sph(10, 18, 260, tgt), tgt, 1.0, (0.9, 1.0, 0.98))
    else:
        area_light("key", 260, 200, sph(205, 50, 330, tgt), tgt, 15.0, (1.0, 0.985, 0.96))
        area_light("top", 420, 420, sph(150, 89, 380, tgt), tgt, 7.0)
        area_light("rim", 200, 60, sph(118, 20, 300, tgt), tgt, 3.6, (0.96, 0.98, 1.0))
        area_light("fill", 260, 200, sph(-40, 30, 330, tgt), tgt, 6.0)


def build_world():
    w = bpy.data.worlds.new("world")
    SCENE.world = w
    w.use_nodes = True
    bg = next(n for n in w.node_tree.nodes if n.type == "BACKGROUND")
    if DARK:
        bg.inputs["Color"].default_value = (0.02, 0.022, 0.022, 1)
        bg.inputs["Strength"].default_value = 0.25
    else:
        bg.inputs["Color"].default_value = (1.0, 1.0, 1.0, 1)
        bg.inputs["Strength"].default_value = 0.12


def build_floor():
    V, F, M = prism(rrect(4000, 4000, 10, 2), -1.0, 0.0)
    mesh_obj("floor", V, F, [MAT["floor"]], smooth=False, recalc=True)


FRAMES = {
    # target, azimuth, elevation, distance, lens, shift x, shift y, f-stop
    "band": dict(target=(11.5, -3.0, 6.8), az=-70.0, el=16.0, dist=186.0, lens=100.0,
                 shift=(0.0, 0.0), fstop=16.0, focus=(15.0, -2.5, 2.0)),
    "square": dict(target=(10.6, -1.8, 6.0), az=-64.0, el=26.0, dist=194.0, lens=100.0,
                   shift=(0.0, 0.0), fstop=16.0, focus=(15.0, -2.5, 2.0)),
}


def build_camera():
    f = dict(FRAMES[ARGS.frame])
    if ARGS.cam:
        for kv in ARGS.cam.split(";"):
            k, v = kv.split("=")
            f[k] = tuple(float(x) for x in v.split(",")) if "," in v else float(v)
    focus_pt = f["focus"]
    tgt = Vector(f["target"])
    pos = sph(f["az"], f["el"], f["dist"], tgt)
    cd = bpy.data.cameras.new("cam")
    cd.lens = f["lens"]
    cd.sensor_width = 36.0
    cd.sensor_fit = "AUTO"
    cd.clip_start = 1.0
    cd.clip_end = 20000.0
    cd.shift_x, cd.shift_y = f["shift"]
    cam = bpy.data.objects.new("cam", cd)
    cam.location = pos
    view = (tgt - pos).normalized()
    cam.rotation_euler = view.to_track_quat("-Z", "Y").to_euler()
    COLL.objects.link(cam)
    SCENE.camera = cam
    cd.dof.use_dof = True
    cd.dof.focus_distance = (Vector(focus_pt) - pos).dot(view)
    # Cycles sizes the aperture in scene units without the unit scale, so the
    # photographic f-number has to be scaled to millimetres by hand.
    cd.dof.aperture_fstop = f["fstop"] * SCENE.unit_settings.scale_length
    cd.dof.aperture_blades = 7
    cd.dof.aperture_rotation = math.radians(12)
    return cam


# ---------------------------------------------------------------- render and compositing

def setup_render():
    r = SCENE.render
    r.engine = "CYCLES"
    r.resolution_x, r.resolution_y = ARGS.w, ARGS.h
    r.resolution_percentage = 100
    r.film_transparent = False
    r.image_settings.file_format = "PNG"
    r.image_settings.color_depth = "16"
    r.image_settings.color_mode = "RGB"
    r.use_compositing = True
    if ARGS.crop:
        x0, y0, x1, y1 = (float(v) for v in ARGS.crop.split(","))
        r.use_border, r.use_crop_to_border = True, True
        r.border_min_x, r.border_min_y, r.border_max_x, r.border_max_y = x0, y0, x1, y1
    if ARGS.threads:
        r.threads_mode = "FIXED"
        r.threads = ARGS.threads
    c = SCENE.cycles
    c.device = "CPU"
    c.samples = ARGS.samples
    c.use_adaptive_sampling = True
    c.adaptive_threshold = 0.012
    c.adaptive_min_samples = min(32, ARGS.samples)
    c.use_denoising = True
    c.denoiser = "OPENIMAGEDENOISE"
    c.denoising_input_passes = "RGB_ALBEDO_NORMAL"
    c.denoising_prefilter = "ACCURATE"
    try:
        c.denoising_quality = "HIGH"
    except (AttributeError, TypeError):
        pass
    c.max_bounces = 10
    c.diffuse_bounces = 4
    c.glossy_bounces = 6
    c.transmission_bounces = 6
    c.transparent_max_bounces = 8
    c.caustics_reflective = False
    c.caustics_refractive = False
    c.sample_clamp_indirect = 8.0
    c.use_light_tree = True
    c.filter_width = 1.4
    vs = SCENE.view_settings
    vs.view_transform = "Khronos PBR Neutral"
    vs.look = "None"
    vs.exposure = 0.0 if DARK else 0.55
    vs.gamma = 1.0
    SCENE.sequencer_colorspace_settings.name = "sRGB"
    SCENE.view_layers[0].use_pass_emit = True


def setup_compositor():
    """Bloom only from what is directly emissive (the signal), added in scene
    linear before the view transform."""
    ng = bpy.data.node_groups.new("compositing", "CompositorNodeTree")
    SCENE.compositing_node_group = ng
    ng.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    N, L = ng.nodes, ng.links
    rl = N.new("CompositorNodeRLayers")
    out = N.new("NodeGroupOutput")
    emit = next(o for o in rl.outputs if o.name in ("Emit", "Emission"))
    cur = rl.outputs["Image"]
    for size, strength in ((0.42, 0.55 if DARK else 0.35), (0.7, 0.35 if DARK else 0.2)):
        g = N.new("CompositorNodeGlare")
        g.inputs["Type"].default_value = "Bloom"
        g.inputs["Quality"].default_value = "High"
        g.inputs["Threshold"].default_value = 0.0
        g.inputs["Smoothness"].default_value = 0.0
        g.inputs["Size"].default_value = size
        g.inputs["Strength"].default_value = 1.0
        L.new(emit, g.inputs["Image"])
        mx = N.new("ShaderNodeMix")
        mx.data_type = "RGBA"
        mx.blend_type = "ADD"
        mx.inputs[0].default_value = strength
        L.new(cur, mx.inputs[6])
        L.new(g.outputs["Glare"], mx.inputs[7])
        cur = mx.outputs[2]
    L.new(cur, out.inputs[0])


# ---------------------------------------------------------------- post

def write_png(path, rgb, bits=16):
    """Write an RGB array (rows bottom to top, values 0..1) as a PNG."""
    import struct
    import zlib
    h, w, _ = rgb.shape
    rgb = np.clip(rgb[::-1], 0, 1)
    if bits == 16:
        data = np.round(rgb * 65535).astype(">u2").reshape(h, -1).view(np.uint8)
    else:
        # No dither: lossy encoders turn dither noise into coloured blotches in
        # near-black areas, and the flattened background has nothing to band.
        data = np.round(rgb * 255).astype(np.uint8).reshape(h, -1)
    raw = np.concatenate([np.zeros((h, 1), np.uint8), data], 1)

    def chunk(t, d):
        return (struct.pack(">I", len(d)) + t + d
                + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF))

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, bits, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw.tobytes(), 6)))
        f.write(chunk(b"IEND", b""))


def _blur2(a, sigma):
    """Separable Gaussian blur of a 2D or 3D array over its first two axes."""
    r = int(3 * sigma) + 1
    k = np.exp(-np.arange(-r, r + 1) ** 2 / (2 * sigma ** 2))
    k /= k.sum()
    for axis in (0, 1):
        pad = [(0, 0)] * a.ndim
        pad[axis] = (r, r)
        p = np.pad(a, pad, mode="edge")
        out = np.zeros_like(a)
        n = a.shape[axis]
        for i, kv in enumerate(k):
            out += kv * (p[i:i + n] if axis == 0 else p[:, i:i + n])
        a = out
    return a


def _upsample(small, h, w):
    """Bilinear resize of a (hs, ws, c) array to (h, w, c)."""
    hs, ws, c = small.shape
    ys = (np.arange(h) + 0.5) * hs / h - 0.5
    xs = (np.arange(w) + 0.5) * ws / w - 0.5
    rows = np.empty((h, ws, c))
    for j in range(ws):
        for ch in range(c):
            rows[:, j, ch] = np.interp(ys, np.arange(hs), small[:, j, ch])
    out = np.empty((h, w, c))
    for i in range(h):
        for ch in range(c):
            out[i, :, ch] = np.interp(xs, np.arange(ws), rows[i, :, ch])
    return out


def backdrop_estimate(a, thr):
    """Smooth estimate of the bare backdrop under the whole frame. Pixels that
    differ from the backdrop by more than thr (the board, its shadow, the glow
    and its spill) are masked out and filled in from their surroundings."""
    h, w, _ = a.shape
    f = 8
    hs, ws = h // f, w // f
    small = a[:hs * f, :ws * f].reshape(hs, f, ws, f, 3).mean((1, 3))
    b = max(2, int(0.02 * min(hs, ws)))
    border = np.concatenate([small[:b].reshape(-1, 3), small[-b:].reshape(-1, 3),
                             small[:, :b].reshape(-1, 3), small[:, -b:].reshape(-1, 3)])
    est = np.broadcast_to(np.median(border, axis=0), small.shape).copy()
    s1, s2 = 0.08 * min(hs, ws), 0.3 * min(hs, ws)
    for _ in range(3):
        bg = (np.abs(small - est).max(-1) < thr).astype(np.float64)
        bg = _blur2(bg, 1.5) > 0.98          # stay clear of the subject's edges
        bg = bg.astype(np.float64)
        n1, d1 = _blur2(small * bg[..., None], s1), _blur2(bg, s1)
        n2, d2 = _blur2(small * bg[..., None], s2), _blur2(bg, s2)
        wgt = np.clip(d1 / 0.3, 0, 1)[..., None]
        est = wgt * n1 / np.maximum(d1, 1e-6)[..., None] + \
            (1 - wgt) * n2 / np.maximum(d2, 1e-6)[..., None]
    return _upsample(est, h, w)


def post_process(raw_path, final_path):
    """Grade the render so the backdrop is exactly the page colour everywhere,
    which lets the image sit on the page with no visible edge or frame. Shadows,
    reflections and glow spill are kept as offsets from the backdrop.
    Returns the graded image (rows bottom to top)."""
    img = bpy.data.images.load(raw_path, check_existing=False)
    img.colorspace_settings.name = "Non-Color"
    w, h = img.size
    a = np.empty(w * h * 4, np.float32)
    img.pixels.foreach_get(a)
    a = a.reshape(h, w, 4)[:, :, :3].astype(np.float64)
    target = np.array([int(PAGE_BG[i:i + 2], 16) for i in (1, 3, 5)], np.float64) / 255.0
    est = backdrop_estimate(a, 0.035 if DARK else 0.02)
    if DARK:
        # lift or lower the dark tones by the backdrop error; highlights untouched
        a = a + (target - est) * np.clip(1 - a / 0.4, 0, 1)
    else:
        a = a * (target / est)
    # Coring: pull what is within a few levels of the page colour exactly onto
    # it, so the encoders get a perfectly flat backdrop instead of +-1 level
    # patches they would turn into blocks. Monotonic, so falloffs stay smooth.
    dl = (a - target) * 255
    m = np.abs(dl).max(-1, keepdims=True)
    k = np.clip((m - 1.0) / 4.0, 0, 1)
    k = k * k * (3 - 2 * k)
    a = target + dl * k / 255
    # Taper whatever still differs from the page colour (glow spill on the
    # floor, bloom halo) to nothing over the outer 12%, so every edge is flat
    # page colour. The backdrop itself is already flat, so this is no vignette.
    yy, xx = np.mgrid[0:h, 0:w]
    d = np.minimum(np.minimum(xx, w - 1 - xx), np.minimum(yy, h - 1 - yy)) / min(w, h)
    t = np.clip(d / 0.12, 0, 1)
    t = (t * t * (3 - 2 * t))[..., None]
    a = np.clip(target + (a - target) * t, 0, 1)
    write_png(final_path, a, 16)
    lo, hi = est.min((0, 1)), est.max((0, 1))
    print(f"post: backdrop {np.round(lo * 255, 1)}..{np.round(hi * 255, 1)} -> "
          f"{np.round(target * 255, 1)}")
    return a


def encode_web(graded, work_dir, web_dir, name):
    """WebP through Pillow (ImageMagick 6 ignores the WebP quality setting),
    lossless unless --webp-q is given, so the backdrop is exactly the page
    colour. AVIF through ImageMagick with full-resolution chroma."""
    src8 = os.path.join(work_dir, name + "-8bit.png")
    write_png(src8, graded, 8)
    os.makedirs(web_dir, exist_ok=True)
    webp = os.path.join(web_dir, name + ".webp")
    avif = os.path.join(web_dir, name + ".avif")
    py = shutil.which("python3")
    opts = "lossless=True, quality=100" if ARGS.webp_q is None else f"quality={ARGS.webp_q}"
    pil = (f"from PIL import Image; Image.open({src8!r}).save({webp!r}, 'WEBP', "
           f"{opts}, method=6)")
    if not py or subprocess.run([py, "-c", pil]).returncode != 0:
        print("encode: python3 with Pillow not found, skipping WebP")
        webp = None
    tool = shutil.which("magick") or shutil.which("convert")
    if tool:
        subprocess.run([tool, src8, "-strip", "-quality", str(ARGS.avif_q),
                        "-define", "heic:chroma=444", "-define", "heic:speed=2", avif],
                       check=True)
    else:
        print("encode: ImageMagick not found, skipping AVIF")
        avif = None
    for p in (webp, avif):
        if p:
            print(f"encode: {p} {os.path.getsize(p) / 1024:.0f} KB")


# ---------------------------------------------------------------- main

def finish(out):
    stem, ext = os.path.splitext(out)
    graded = post_process(out, stem + "-final.png")
    if ARGS.web:
        encode_web(graded, os.path.dirname(out), os.path.abspath(ARGS.web),
                   os.path.basename(stem))


def main():
    if ARGS.post_only:
        finish(os.path.abspath(ARGS.out))
        return
    t0 = time.time()
    setup_render()
    # Geometry first fills the copper map; the mask material then reads it.
    # Materials are needed by the geometry, so build them against a stand-in
    # image and swap the real copper map in afterwards.
    placeholder = bpy.data.images.new("copper_placeholder", 4, 4, float_buffer=True)
    build_materials(placeholder)
    build_board()
    build_signal()
    build_layout()
    build_silk()
    img = CU.image()
    for n in MAT["mask"].node_tree.nodes:
        if n.type == "TEX_IMAGE":
            n.image = img
    build_floor()
    build_lights()
    build_world()
    build_camera()
    setup_compositor()
    print(f"scene built in {time.time() - t0:.1f}s")
    if ARGS.blend:
        bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(ARGS.blend))
    out = os.path.abspath(ARGS.out)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    SCENE.render.filepath = out
    t1 = time.time()
    bpy.ops.render.render(write_still=True)
    print(f"render {ARGS.variant}/{ARGS.frame} {ARGS.w}x{ARGS.h} {ARGS.samples} spp: "
          f"{time.time() - t1:.0f}s")
    if ARGS.crop:
        return
    finish(out)


main()
