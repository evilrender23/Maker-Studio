#!/usr/bin/env python3
"""Importa los packs CC0 de Quaternius a la biblioteca integrada de Maker Studio.

Uso (desde la raíz del repo):

    uv run --with pillow python3 -I scripts/importar_quaternius.py ORIGEN [--destino assets/biblioteca]
        [--sin-miniaturas] [--solo PAQUETE,...]

ORIGEN es una carpeta con los .zip descargados de quaternius.itch.io o con
esos zips ya descomprimidos (una carpeta por pack). Solo se usan los glTF/GLB.

Por cada modelo escribe un .glb autocontenido en geometría que referencia
texturas compartidas del pack (`<pack>/texturas/*.jpg|png`):
  - solo se conserva el color base (sin normales, ORM, ni UV/colores extra);
  - texturas reducidas a 2048 px como máximo (JPEG calidad 92, color sin submuestrear),
    JPEG salvo las que necesitan alfa (PNG con los bordes transparentes
    rellenados para que el filtrado no oscurezca las hojas);
  - texturas deduplicadas por contenido dentro de cada pack;
  - colores de vértice blancos eliminados y el resto a 8 bits;
  - animaciones con las claves redundantes eliminadas.

Además escribe `catalogo.json`, copia las licencias a `LICENCIAS/` y, salvo
`--sin-miniaturas`, renderiza las miniaturas con el renderizador del motor
(`cargo run --release -p maker_render --example miniaturas`).
"""

import argparse
import array
import hashlib
import io
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile

from PIL import Image, ImageChops, ImageFilter, ImageStat

AUTOR = "Quaternius"
LICENCIA = "CC0 1.0 Universal"

# id, carpeta del pack, nombre, slug de itch.io, carpetas de glTF a incluir
# (componentes exactos de la ruta), carpetas a excluir.
PAQUETES = [
    ("naturaleza", "Stylized Nature MegaKit[Standard]", "Stylized Nature MegaKit", "stylized-nature-megakit", ["glTF"], []),
    ("medieval", "Medieval Village MegaKit[Standard]", "Medieval Village MegaKit", "medieval-village-megakit", ["glTF"], []),
    ("ciudad", "Downtown City MegaKit[Standard]", "Downtown City MegaKit", "downtown-city-megakit", ["glTF (Godot)"], []),
    ("props_fantasia", "Fantasy Props MegaKit[Standard]", "Fantasy Props MegaKit", "fantasy-props-megakit", ["glTF"], []),
    ("scifi_modular", "Modular SciFi MegaKit[Standard]", "Modular SciFi MegaKit", "modular-sci-fi-megakit", ["glTF"], []),
    ("scifi_esencial", "Sci-Fi Essentials Kit[Standard]", "Sci-Fi Essentials Kit", "sci-fi-essentials-kit", ["glTF"], []),
    # Los peinados vienen dos veces: «Origin at 0» son las mismas mallas sin
    # esqueleto. Se queda la versión con huesos, que se anima con el personaje.
    ("personajes", "Universal Base Characters[Standard]", "Universal Base Characters", "universal-base-characters", ["Godot - UE", "glTF (Godot -Unreal)"], ["Origin at 0"]),
    ("atuendos", "Modular Character Outfits - Fantasy[Standard]", "Modular Character Outfits - Fantasy", "modular-character-outfits-fantasy", ["glTF (Godot-Unreal)"], []),
    ("animaciones", "Universal Animation Library[Standard]", "Universal Animation Library", "universal-animation-library", ["Unreal-Godot"], []),
    ("animaciones2", "Universal Animation Library 2[Standard]", "Universal Animation Library 2", "universal-animation-library-2", ["Unreal-Godot"], []),
]

TEX_MAX = 2048
JPEG_CALIDAD = 92

# ───────────────────────────── categorías ─────────────────────────────

REGLAS = {
    "naturaleza": [
        (r"Tree|Pine", "Árboles"), (r"Bush|Fern|Plant", "Arbustos"), (r"Grass|Clover", "Hierba"),
        (r"Flower", "Flores"), (r"Mushroom", "Setas"), (r"Rock|Pebble", "Rocas"),
    ],
    "medieval": [
        (r"^Wall", "Paredes"), (r"^Roof|Chimney", "Tejados"), (r"^Floor", "Suelos"), (r"Door", "Puertas"),
        (r"Window", "Ventanas"), (r"Stair", "Escaleras"), (r"^Prop", "Props"),
    ],
    "ciudad": [
        (r"^Street|^Sidewalk|^Decal", "Calles"), (r"^Roof", "Tejados"), (r"^Building", "Edificios"),
        (r"^Prop", "Props"), (r"Door|Entrance|Stairs", "Puertas"),
    ],
    "props_fantasia": [
        (r"Axe|Sword|Shield|Pickaxe|WeaponStand|Anvil|Whetstone", "Armas y herramientas"),
        (r"Bed|Bench|Chair|Table_Large|Stool|Shelf|Bookcase|Cabinet|Nightstand|Workbench|Stall|Peg_Rack|Dummy", "Muebles"),
        (r"Barrel|Crate|Chest|Bag|Pouch|Bucket|Pot_|Vase|Cauldron|Cage", "Contenedores"),
        (r"Candle|Lantern|Torch|Chandelier", "Luces"),
        (r"Book|Scroll", "Libros"),
        (r"Banner", "Estandartes"),
    ],
    "scifi_esencial": [(r"^Enemy", "Enemigos"), (r"^Gun", "Armas"), (r"^Prop", "Props")],
    "personajes": [(r"FullBody", "Personajes"), (r"Hair|Eyebrows", "Pelo")],
    "animaciones": [(r".", "Maniquí")],
    "animaciones2": [(r".", "Maniquí")],
}

CARPETAS_CATEGORIA = {
    "Aliens": "Alienígenas", "Columns": "Columnas", "Decals": "Calcomanías", "Platforms": "Plataformas",
    "Props": "Props", "Walls": "Paredes", "Outfits": "Atuendos", "Modular Parts": "Piezas",
}


def categoria(pack, rel_dir, stem):
    for parte in reversed(rel_dir.split("/")):
        if parte in CARPETAS_CATEGORIA:
            return CARPETAS_CATEGORIA[parte]
    for patron, cat in REGLAS.get(pack, []):
        if re.search(patron, stem):
            return cat
    return {"medieval": "Estructura", "ciudad": "Fachadas"}.get(pack, "Objetos")


def nombre_visible(stem):
    s = stem.replace("_", " ").replace("-", " ")
    s = re.sub(r"(?<=[a-z])(?=[A-Z])", " ", s)
    return re.sub(r"\s+", " ", s).strip()


# ───────────────────────────── lectura glTF ─────────────────────────────

CT_FMT = {5120: "b", 5121: "B", 5122: "h", 5123: "H", 5125: "I", 5126: "f"}
CT_SIZE = {5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT2": 4, "MAT3": 9, "MAT4": 16}


def cargar(path):
    if path.lower().endswith(".glb"):
        data = open(path, "rb").read()
        magic, _ver, _len = struct.unpack_from("<III", data, 0)
        if magic != 0x46546C67:
            raise ValueError("GLB inválido")
        off = 12
        j, bins = None, []
        while off < len(data):
            clen, ctype = struct.unpack_from("<II", data, off)
            chunk = data[off + 8: off + 8 + clen]
            if ctype == 0x4E4F534A:
                j = json.loads(chunk)
            elif ctype == 0x004E4942:
                bins.append(chunk)
            off += 8 + clen
        bufs = []
        for b in j.get("buffers", []):
            if "uri" in b:
                bufs.append(open(os.path.join(os.path.dirname(path), b["uri"]), "rb").read())
            else:
                bufs.append(bins[0])
        return j, bufs
    j = json.load(open(path, encoding="utf-8"))
    bufs = []
    for b in j.get("buffers", []):
        uri = b["uri"]
        if uri.startswith("data:"):
            import base64
            bufs.append(base64.b64decode(uri.split(",", 1)[1]))
        else:
            from urllib.parse import unquote
            bufs.append(open(os.path.join(os.path.dirname(path), unquote(uri)), "rb").read())
    return j, bufs


def leer_accessor(j, bufs, idx):
    """Bytes compactos (sin stride) + descriptor."""
    a = j["accessors"][idx]
    if "sparse" in a:
        raise ValueError("accessor disperso no soportado")
    n = NCOMP[a["type"]]
    cs = CT_SIZE[a["componentType"]]
    elem = n * cs
    count = a["count"]
    if "bufferView" not in a:
        return bytes(elem * count), a
    bv = j["bufferViews"][a["bufferView"]]
    buf = bufs[bv["buffer"]]
    base = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride", 0) or elem
    if stride == elem:
        return bytes(buf[base: base + elem * count]), a
    out = bytearray(elem * count)
    for i in range(count):
        out[i * elem:(i + 1) * elem] = buf[base + i * stride: base + i * stride + elem]
    return bytes(out), a


def como_floats(raw, a):
    arr = array.array(CT_FMT[a["componentType"]])
    arr.frombytes(raw)
    if a["componentType"] == 5126:
        return list(arr)
    if a.get("normalized"):
        m = {5121: 255.0, 5123: 65535.0, 5120: 127.0, 5122: 32767.0}[a["componentType"]]
        return [max(v / m, -1.0) for v in arr]
    return [float(v) for v in arr]


# ───────────────────────────── escritura GLB ─────────────────────────────

class Salida:
    def __init__(self):
        self.bin = bytearray()
        self.views = []
        self.accessors = []
        self.cache = {}

    def accessor(self, raw, comp, typ, count, target=None, normalized=False, minmax=None, reuse=True):
        key = (hashlib.md5(raw).digest(), comp, typ, count, target, normalized) if reuse else None
        if key is not None and key in self.cache:
            return self.cache[key]
        while len(self.bin) % 4:
            self.bin.append(0)
        view = {"buffer": 0, "byteOffset": len(self.bin), "byteLength": len(raw)}
        if target:
            view["target"] = target
        self.bin.extend(raw)
        self.views.append(view)
        acc = {"bufferView": len(self.views) - 1, "componentType": comp, "count": count, "type": typ}
        if normalized:
            acc["normalized"] = True
        if minmax:
            acc["min"], acc["max"] = minmax
        self.accessors.append(acc)
        idx = len(self.accessors) - 1
        if key is not None:
            self.cache[key] = idx
        return idx

    def glb(self, j):
        while len(self.bin) % 4:
            self.bin.append(0)
        j["buffers"] = [{"byteLength": len(self.bin)}]
        j["bufferViews"] = self.views
        j["accessors"] = self.accessors
        js = json.dumps(j, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        while len(js) % 4:
            js += b" "
        total = 12 + 8 + len(js) + 8 + len(self.bin)
        out = bytearray(struct.pack("<III", 0x46546C67, 2, total))
        out += struct.pack("<II", len(js), 0x4E4F534A) + js
        out += struct.pack("<II", len(self.bin), 0x004E4942) + self.bin
        return bytes(out)


def minmax_vec3(vals):
    xs, ys, zs = vals[0::3], vals[1::3], vals[2::3]
    return [min(xs), min(ys), min(zs)], [max(xs), max(ys), max(zs)]


# ───────────────────────────── animaciones ─────────────────────────────

def nlerp(a, b, t):
    if sum(x * y for x, y in zip(a, b)) < 0:
        b = [-x for x in b]
    r = [x + (y - x) * t for x, y in zip(a, b)]
    n = math.sqrt(sum(x * x for x in r)) or 1.0
    return [x / n for x in r]


def reducir_claves(times, vals, n, rot, tol):
    """Quita las claves que la interpolación lineal ya reproduce."""
    k = len(times)
    keys = [vals[i * n:(i + 1) * n] for i in range(k)]
    if all(max(abs(x - y) for x, y in zip(keys[0], kk)) < 1e-6 for kk in keys):
        return [times[0]], keys[0]
    keep = [0]
    for i in range(1, k - 1):
        a = keep[-1]
        t = (times[i] - times[a]) / max(times[i + 1] - times[a], 1e-9)
        guess = nlerp(keys[a], keys[i + 1], t) if rot else [x + (y - x) * t for x, y in zip(keys[a], keys[i + 1])]
        # Todas las claves saltadas desde `a` deben seguir bien aproximadas.
        ok = max(abs(x - y) for x, y in zip(guess, keys[i])) < tol
        if ok:
            for m in range(a + 1, i):
                tm = (times[m] - times[a]) / max(times[i + 1] - times[a], 1e-9)
                g = nlerp(keys[a], keys[i + 1], tm) if rot else [x + (y - x) * tm for x, y in zip(keys[a], keys[i + 1])]
                if max(abs(x - y) for x, y in zip(g, keys[m])) >= tol:
                    ok = False
                    break
        if not ok:
            keep.append(i)
    keep.append(k - 1)
    out = []
    for i in keep:
        out.extend(keys[i])
    return [times[i] for i in keep], out


# ───────────────────────────── texturas ─────────────────────────────

def rellenar_transparentes(im):
    """Empuja el color de los píxeles opacos hacia los transparentes (push-pull)
    para que el filtrado y los mipmaps no traigan halos oscuros."""
    alpha = im.getchannel("A")
    known = alpha.point(lambda a: 255 if a > 8 else 0)
    filled = im.copy()
    prem = im.convert("RGBa")
    for r in (2, 6, 18, 54, 160):
        blur = prem.filter(ImageFilter.GaussianBlur(r)).convert("RGBA")
        reach = blur.getchannel("A").point(lambda a: 255 if a > 0 else 0)
        nuevos = ImageChops.subtract(reach, known)
        filled.paste(blur, mask=nuevos)
        known = ImageChops.lighter(known, reach)
    filled.putalpha(alpha)
    return filled


def reducir(im, alpha):
    w, h = im.size
    escala = min(1.0, TEX_MAX / max(w, h))
    if escala < 1.0:
        im = im.resize((max(1, round(w * escala)), max(1, round(h * escala))), Image.LANCZOS)
    return im


class Texturas:
    """Texturas de un pack: deduplicadas por contenido."""

    def __init__(self, dir_out):
        self.dir = dir_out
        self.por_hash = {}
        self.nombres = set()
        self.bytes = 0

    def anadir(self, src):
        data = open(src, "rb").read()
        key = hashlib.md5(data).hexdigest()
        if key in self.por_hash:
            return self.por_hash[key]
        im = Image.open(io.BytesIO(data))
        im.load()
        # El alfa se conserva si la imagen lo usa de verdad (hojas, calcomanías).
        rgba = im.convert("RGBA")
        lo, _hi = rgba.getchannel("A").getextrema()
        tiene_alfa = lo < 250
        stem = re.sub(r"[^A-Za-z0-9_]+", "_", os.path.splitext(os.path.basename(src))[0]).strip("_")
        stem = re.sub(r"_png$", "", stem)
        nombre = stem
        i = 2
        while nombre in self.nombres:
            nombre = f"{stem}_{i}"
            i += 1
        self.nombres.add(nombre)
        os.makedirs(self.dir, exist_ok=True)
        if tiene_alfa:
            rgba = rellenar_transparentes(rgba)
            rgba = reducir(rgba, True)
            fn = nombre + ".png"
            rgba.save(os.path.join(self.dir, fn), optimize=True)
        else:
            rgb = reducir(im.convert("RGB"), False)
            fn = nombre + ".jpg"
            rgb.save(os.path.join(self.dir, fn), quality=JPEG_CALIDAD, optimize=True, progressive=False, subsampling="4:4:4")
        self.bytes += os.path.getsize(os.path.join(self.dir, fn))
        self.por_hash[key] = fn
        return fn


def buscar_imagen(uri, gltf_dir, pack_dir):
    from urllib.parse import unquote
    uri = unquote(uri)
    p = os.path.join(gltf_dir, uri)
    if os.path.exists(p):
        return p
    base = os.path.basename(uri)
    for dp, _dn, fn in os.walk(pack_dir):
        if base in fn:
            return os.path.join(dp, base)
    return None


# ───────────────────────────── conversión de un modelo ─────────────────────────────

def convertir(path, pack_dir, texturas):
    j, bufs = cargar(path)
    out = Salida()
    gltf_dir = os.path.dirname(path)
    res = {"asset": {"version": "2.0", "generator": "Maker Studio importar_quaternius.py"}}
    res["scene"] = j.get("scene", 0)
    res["scenes"] = [{"nodes": s.get("nodes", [])} for s in j.get("scenes", [])]
    nodes = []
    for n in j.get("nodes", []):
        m = {k: n[k] for k in ("name", "children", "mesh", "skin", "translation", "rotation", "scale", "matrix") if k in n}
        nodes.append(m)
    res["nodes"] = nodes

    # Materiales: solo color base.
    mats, images, textures = [], [], []
    img_index = {}
    uv_de_material = {}
    for mi, m in enumerate(j.get("materials", [])):
        pbr = m.get("pbrMetallicRoughness", {})
        alpha_mode = m.get("alphaMode", "OPAQUE")
        nm = {"name": m.get("name", f"mat{mi}")}
        p = {}
        if "baseColorFactor" in pbr:
            p["baseColorFactor"] = [round(v, 5) for v in pbr["baseColorFactor"]]
        p["metallicFactor"] = pbr.get("metallicFactor", 1.0)
        p["roughnessFactor"] = pbr.get("roughnessFactor", 1.0)
        bct = pbr.get("baseColorTexture")
        if bct is not None:
            tex = j["textures"][bct["index"]]
            src_img = j["images"][tex["source"]]
            if "uri" not in src_img:
                raise ValueError("imagen incrustada no soportada")
            src = buscar_imagen(src_img["uri"], gltf_dir, pack_dir)
            if src is None:
                print(f"    aviso: falta la imagen {src_img['uri']}")
            else:
                fn = texturas.anadir(src)
                if fn not in img_index:
                    img_index[fn] = len(images)
                    images.append({"uri": "texturas/" + fn})
                    textures.append({"sampler": 0, "source": img_index[fn]})
                p["baseColorTexture"] = {"index": img_index[fn]}
                uv_de_material[mi] = bct.get("texCoord", 0)
        nm["pbrMetallicRoughness"] = p
        if alpha_mode != "OPAQUE":
            nm["alphaMode"] = alpha_mode
            if alpha_mode == "MASK":
                nm["alphaCutoff"] = m.get("alphaCutoff", 0.5)
        if m.get("doubleSided"):
            nm["doubleSided"] = True
        mats.append(nm)
    if mats:
        res["materials"] = mats
    if images:
        res["images"] = images
        res["textures"] = textures
        res["samplers"] = [{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]

    # Mallas.
    tris = 0
    meshes = []
    for me in j.get("meshes", []):
        prims = []
        for pr in me["primitives"]:
            mode = pr.get("mode", 4)
            if mode != 4:
                continue
            at = pr["attributes"]
            nat = {}
            raw, a = leer_accessor(j, bufs, at["POSITION"])
            vals = como_floats(raw, a)
            nat["POSITION"] = out.accessor(array.array("f", vals).tobytes(), 5126, "VEC3", a["count"], 34962, minmax=minmax_vec3(vals))
            count = a["count"]
            if "NORMAL" in at:
                raw, a = leer_accessor(j, bufs, at["NORMAL"])
                nat["NORMAL"] = out.accessor(array.array("f", como_floats(raw, a)).tobytes(), 5126, "VEC3", count, 34962)
            uv = uv_de_material.get(pr.get("material"), 0)
            if f"TEXCOORD_{uv}" in at:
                raw, a = leer_accessor(j, bufs, at[f"TEXCOORD_{uv}"])
                nat["TEXCOORD_0"] = out.accessor(array.array("f", como_floats(raw, a)).tobytes(), 5126, "VEC2", count, 34962)
            if "COLOR_0" in at:
                raw, a = leer_accessor(j, bufs, at["COLOR_0"])
                f = como_floats(raw, a)
                nc = NCOMP[a["type"]]
                if any(v < 0.995 for v in f):
                    if nc == 3:
                        f4 = []
                        for i in range(0, len(f), 3):
                            f4.extend(f[i:i + 3] + [1.0])
                        f = f4
                    b = bytes(max(0, min(255, round(v * 255))) for v in f)
                    nat["COLOR_0"] = out.accessor(b, 5121, "VEC4", count, 34962, normalized=True)
            if "JOINTS_0" in at and "WEIGHTS_0" in at:
                raw, a = leer_accessor(j, bufs, at["JOINTS_0"])
                nat["JOINTS_0"] = out.accessor(raw, a["componentType"], "VEC4", count, 34962)
                raw, a = leer_accessor(j, bufs, at["WEIGHTS_0"])
                w = como_floats(raw, a)
                q = []
                for i in range(0, len(w), 4):
                    s = sum(w[i:i + 4]) or 1.0
                    q.extend(max(0, min(65535, round(v / s * 65535))) for v in w[i:i + 4])
                nat["WEIGHTS_0"] = out.accessor(array.array("H", q).tobytes(), 5123, "VEC4", count, 34962, normalized=True)
            np_ = {"attributes": nat}
            if "indices" in pr:
                raw, a = leer_accessor(j, bufs, pr["indices"])
                idx = array.array(CT_FMT[a["componentType"]])
                idx.frombytes(raw)
                if a["componentType"] == 5121:
                    idx = array.array("H", idx)
                comp = 5123 if count < 65536 else 5125
                idx = array.array("H" if comp == 5123 else "I", idx)
                np_["indices"] = out.accessor(idx.tobytes(), comp, "SCALAR", len(idx), 34963)
                tris += len(idx) // 3
            else:
                tris += count // 3
            if "material" in pr:
                np_["material"] = pr["material"]
            prims.append(np_)
        meshes.append({"name": me.get("name", ""), "primitives": prims})
    res["meshes"] = meshes

    # Pieles.
    skins = []
    for s in j.get("skins", []):
        ns = {"joints": s["joints"]}
        if "inverseBindMatrices" in s:
            raw, a = leer_accessor(j, bufs, s["inverseBindMatrices"])
            ns["inverseBindMatrices"] = out.accessor(raw, 5126, "MAT4", a["count"])
        if "skeleton" in s:
            ns["skeleton"] = s["skeleton"]
        skins.append(ns)
    if skins:
        res["skins"] = skins

    # Animaciones con claves reducidas.
    anims = []
    keys_in = keys_out = 0
    for an in j.get("animations", []):
        samplers, channels = [], []
        for ch in an["channels"]:
            smp = an["samplers"][ch["sampler"]]
            path_ = ch["target"]["path"]
            if path_ == "weights":
                continue
            interp = smp.get("interpolation", "LINEAR")
            raw_t, at_ = leer_accessor(j, bufs, smp["input"])
            times = como_floats(raw_t, at_)
            raw_v, av = leer_accessor(j, bufs, smp["output"])
            vals = como_floats(raw_v, av)
            n = 4 if path_ == "rotation" else 3
            keys_in += len(times)
            if interp == "LINEAR" and len(times) > 2:
                tol = 2e-4 if path_ == "rotation" else 2e-4
                times, vals = reducir_claves(times, vals, n, path_ == "rotation", tol)
            elif interp == "STEP" and len(times) > 1:
                keys = [tuple(vals[i * n:(i + 1) * n]) for i in range(len(times))]
                if all(k == keys[0] for k in keys):
                    times, vals = [times[0]], list(keys[0])
            keys_out += len(times)
            ti = out.accessor(array.array("f", times).tobytes(), 5126, "SCALAR", len(times), minmax=([min(times)], [max(times)]))
            vi = out.accessor(array.array("f", vals).tobytes(), 5126, "VEC4" if n == 4 else "VEC3", len(vals) // n, reuse=False)
            samplers.append({"input": ti, "output": vi, "interpolation": interp})
            channels.append({"sampler": len(samplers) - 1, "target": {"node": ch["target"]["node"], "path": path_}})
        if channels:
            anims.append({"name": an.get("name", f"anim{len(anims)}"), "samplers": samplers, "channels": channels})
    if anims:
        res["animations"] = anims
    return out.glb(res), {
        "triangles": tris,
        "has_skin": bool(skins),
        "has_animations": bool(anims),
        "clips": [a["name"] for a in anims],
        "keys": (keys_in, keys_out),
    }


# ───────────────────────────── principal ─────────────────────────────

def preparar_origen(origen, tmp):
    """Devuelve la carpeta con los packs descomprimidos."""
    zips = [f for f in os.listdir(origen) if f.lower().endswith(".zip")]
    if not zips:
        return origen
    for z in zips:
        dest = os.path.join(tmp, os.path.splitext(z)[0])
        if not os.path.exists(dest):
            print(f"descomprimiendo {z}…")
            with zipfile.ZipFile(os.path.join(origen, z)) as zf:
                zf.extractall(dest)
    return tmp


def buscar_pack(raiz, carpeta):
    p = os.path.join(raiz, carpeta)
    if os.path.isdir(p):
        return p
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("origen")
    ap.add_argument("--destino", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "biblioteca"))
    ap.add_argument("--sin-miniaturas", action="store_true")
    ap.add_argument("--solo", default="")
    args = ap.parse_args()
    destino = os.path.abspath(args.destino)
    solo = set(s for s in args.solo.split(",") if s)
    tmp = tempfile.mkdtemp(prefix="maker-quaternius-")
    try:
        raiz = preparar_origen(os.path.abspath(args.origen), tmp)
        catalogo_path = os.path.join(destino, "catalogo.json")
        previo = {"packs": [], "models": []}
        if solo and os.path.exists(catalogo_path):
            previo = json.load(open(catalogo_path, encoding="utf-8"))
        packs_out = [p for p in previo["packs"] if p["id"] not in solo]
        models_out = [m for m in previo["models"] if m["pack"] not in solo]
        os.makedirs(os.path.join(destino, "LICENCIAS"), exist_ok=True)
        resumen = []
        for pid, carpeta, nombre, slug, incluir, excluir in PAQUETES:
            if solo and pid not in solo:
                continue
            pack_dir = buscar_pack(raiz, carpeta)
            if pack_dir is None:
                print(f"[{pid}] no encontrado en {raiz}: se omite")
                resumen.append((pid, 0, 0, "no encontrado"))
                continue
            fuente = f"https://quaternius.itch.io/{slug}"
            out_dir = os.path.join(destino, pid)
            shutil.rmtree(out_dir, ignore_errors=True)
            os.makedirs(out_dir)
            # Licencia.
            for dp, _dn, fn in os.walk(pack_dir):
                for f in fn:
                    if f.lower().startswith("license") and f.lower().endswith(".txt"):
                        shutil.copy(os.path.join(dp, f), os.path.join(destino, "LICENCIAS", f"{pid}.txt"))
            texturas = Texturas(os.path.join(out_dir, "texturas"))
            fuentes, omitidos = [], []
            for dp, dn, fn in os.walk(pack_dir):
                dn.sort()
                rel = os.path.relpath(dp, pack_dir).replace("\\", "/")
                partes = rel.split("/")
                for f in sorted(fn):
                    if not f.lower().endswith((".gltf", ".glb")):
                        continue
                    if any(x in partes for x in excluir):
                        omitidos.append((f, "duplicado sin esqueleto"))
                        continue
                    if not any(x in partes for x in incluir):
                        omitidos.append((f, "fuera de las carpetas glTF elegidas"))
                        continue
                    fuentes.append((os.path.join(dp, f), rel))
            vistos = set()
            n_ok = 0
            for src, rel in fuentes:
                stem = os.path.splitext(os.path.basename(src))[0]
                if stem in vistos:
                    omitidos.append((stem, "nombre repetido"))
                    continue
                vistos.add(stem)
                try:
                    glb, info = convertir(src, pack_dir, texturas)
                except FileNotFoundError as e:
                    omitidos.append((stem, f"el pack original no trae {os.path.basename(e.filename)}"))
                    continue
                except Exception as e:  # noqa: BLE001
                    omitidos.append((stem, f"error: {e}"))
                    print(f"  ✗ {stem}: {e}")
                    continue
                with open(os.path.join(out_dir, stem + ".glb"), "wb") as fh:
                    fh.write(glb)
                n_ok += 1
                models_out.append({
                    "pack": pid,
                    "category": categoria(pid, rel, stem),
                    "name": nombre_visible(stem),
                    "path": f"{pid}/{stem}.glb",
                    "thumbnail": f"{pid}/miniaturas/{stem}.png",
                    "triangles": info["triangles"],
                    "has_skin": info["has_skin"],
                    "has_animations": info["has_animations"],
                    "clips": info["clips"],
                    "source": fuente,
                })
                if info["keys"][0]:
                    print(f"  {stem}: claves de animación {info['keys'][0]} → {info['keys'][1]}")
            geo = sum(os.path.getsize(os.path.join(out_dir, f)) for f in os.listdir(out_dir) if f.endswith(".glb"))
            print(f"[{pid}] {n_ok} modelos, {len(omitidos)} omitidos, geometría {geo / 1e6:.1f} MB, texturas {texturas.bytes / 1e6:.1f} MB ({len(texturas.nombres)})")
            for o in omitidos:
                print(f"    omitido {o[0]}: {o[1]}")
            packs_out.append({"id": pid, "name": nombre, "author": AUTOR, "license": LICENCIA, "source": fuente, "models": n_ok})
            resumen.append((pid, n_ok, len(omitidos), ""))
        orden = [p[0] for p in PAQUETES]
        packs_out.sort(key=lambda p: orden.index(p["id"]) if p["id"] in orden else 99)
        models_out.sort(key=lambda m: (orden.index(m["pack"]) if m["pack"] in orden else 99, m["category"], m["name"]))
        with open(catalogo_path, "w", encoding="utf-8") as fh:
            json.dump({"version": 1, "packs": packs_out, "models": models_out}, fh, ensure_ascii=False, indent=1)
            fh.write("\n")
        print("\nResumen (importados / omitidos):")
        for pid, ok, om, nota in resumen:
            print(f"  {pid:16} {ok:4} / {om:4} {nota}")
        if not args.sin_miniaturas:
            repo = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
            print("\nRenderizando miniaturas…")
            cmd = ["cargo", "run", "--release", "-p", "maker_render", "--example", "miniaturas", "--", destino]
            if solo:
                cmd += ["--solo", ",".join(sorted(solo))]
            subprocess.run(cmd, cwd=repo, check=True)
        total = 0
        for dp, _dn, fn in os.walk(destino):
            total += sum(os.path.getsize(os.path.join(dp, f)) for f in fn)
        print(f"\nBiblioteca: {total / 1e6:.1f} MB en {destino}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
