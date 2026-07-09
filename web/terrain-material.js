// PBR terrain material. Weights come from the per-tile class textures
// (class.bin → top-4 class indices + weights) driving texture ARRAYS with
// one layer per uploaded material; classes without materials render with
// their average color. All sampling is stochastic (per-cell random
// offsets over a triangular grid) so repeats never show. Falls back to a
// procedural slope/height look while class data is loading.
//
// Water is the sea plane at exactly 0 m: sunk WATER_DROP in the vertex
// stage so tidal-zone terrain stands above the surface.

import * as THREE from 'three';

const WATER_DROP = 0.7; // meters the water plane is lowered in the viewer

const MAXC = 32; // max class ids addressed by the shader
const TEX = 1024; // material layer size (server stores 1024²)

// Legacy fixed sets — the procedural fallback look.
const MATERIALS = {
  uGAc: ['Ground003_2K-PNG_Color.png', true],
  uGAn: ['Ground003_2K-PNG_NormalGL.png', false],
  uGAr: ['Ground003_2K-PNG_Roughness.png', false],
  uGAd: ['Ground003_2K-PNG_Displacement.png', false],
  uGBc: ['Ground037_2K-PNG_Color.png', true],
  uGBn: ['Ground037_2K-PNG_NormalGL.png', false],
  uGBr: ['Ground037_2K-PNG_Roughness.png', false],
  uGBd: ['Ground037_2K-PNG_Displacement.png', false],
  uRc: ['Rock051_2K-PNG_Color.png', true],
  uRn: ['Rock051_2K-PNG_NormalGL.png', false],
  uRr: ['Rock051_2K-PNG_Roughness.png', false],
  uRd: ['Rock051_2K-PNG_Displacement.png', false],
};

let textures = null;

// Neutral 1x1 placeholder so the material renders sanely before download:
// mid-gray for color/roughness/displacement, flat (128,128,255) for normals.
function placeholder(key) {
  const px = key.endsWith('n') ? [128, 128, 255, 255] : [128, 128, 128, 255];
  const t = new THREE.DataTexture(new Uint8Array(px), 1, 1);
  t.needsUpdate = true;
  return t;
}

// 2K sources are resized to 1K on the client: 4x less VRAM (the full set
// froze weaker machines) while 1K / 5 m is still ~5 mm per texel.
function loadTextures() {
  if (textures) return textures;
  textures = {};
  for (const [key, [file, srgb]] of Object.entries(MATERIALS)) {
    const t = placeholder(key);
    t.wrapS = t.wrapT = THREE.RepeatWrapping;
    t.anisotropy = 2;
    if (srgb) t.colorSpace = THREE.SRGBColorSpace;
    t.generateMipmaps = true;
    textures[key] = t;
    fetch(`/materials/${file}`)
      .then((r) => r.blob())
      .then((b) => createImageBitmap(b, { resizeWidth: 1024, resizeHeight: 1024, resizeQuality: 'high' }))
      .then((img) => {
        t.image = img;
        t.needsUpdate = true;
      })
      .catch(() => console.warn(`materiale ${file} kunne ikke lastes`));
  }
  return textures;
}

/* ---------- class material arrays ---------- */

// { alb, nrm, rgh: DataArrayTexture, mats: Float32Array(MAXC*4),
//   tints: Float32Array(MAXC*3) } — one layer per uploaded material.
let classState = null;
const registry = new Set(); // live materials to update when classes load

function hexToRgb(s) {
  const v = parseInt((s || '#5a6450').replace('#', ''), 16);
  return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255];
}

function makeArray(layers, srgb) {
  const t = new THREE.DataArrayTexture(
    new Uint8Array(TEX * TEX * 4 * Math.max(1, layers)),
    TEX,
    TEX,
    Math.max(1, layers),
  );
  t.wrapS = t.wrapT = THREE.RepeatWrapping;
  t.minFilter = THREE.LinearMipmapLinearFilter;
  t.magFilter = THREE.LinearFilter;
  t.generateMipmaps = true;
  t.anisotropy = 2;
  if (srgb) t.colorSpace = THREE.SRGBColorSpace;
  t.needsUpdate = true;
  return t;
}

async function layerInto(tex, layer, url, fallback) {
  try {
    const res = await fetch(url);
    if (!res.ok) throw new Error(res.status);
    const bmp = await createImageBitmap(await res.blob(), {
      resizeWidth: TEX,
      resizeHeight: TEX,
      resizeQuality: 'high',
    });
    const c = new OffscreenCanvas(TEX, TEX);
    const ctx = c.getContext('2d');
    ctx.drawImage(bmp, 0, 0);
    const data = ctx.getImageData(0, 0, TEX, TEX).data; // opaque → no premultiply loss
    tex.image.data.set(data, layer * TEX * TEX * 4);
    tex.needsUpdate = true;
  } catch {
    if (fallback) {
      const px = new Uint8Array(TEX * TEX * 4);
      for (let i = 0; i < px.length; i += 4) px.set(fallback, i);
      tex.image.data.set(px, layer * TEX * TEX * 4);
      tex.needsUpdate = true;
    }
  }
}

/// Fetch the project's classes and build the shader-side material tables.
/// Call once when the viewer starts (and again after class edits).
export async function initClasses() {
  let classes;
  try {
    const res = await fetch('/api/classes');
    if (!res.ok) return;
    classes = (await res.json()).classes || [];
  } catch {
    return;
  }
  const layerOf = new Map(); // material dir -> array layer
  for (const c of classes) {
    for (const m of c.materials) {
      if (!layerOf.has(m.dir)) layerOf.set(m.dir, layerOf.size);
    }
  }
  const n = layerOf.size;
  const alb = makeArray(n, true);
  const nrm = makeArray(n, false);
  const rgh = makeArray(n, false);
  for (const [dir, layer] of layerOf) {
    layerInto(alb, layer, `/data/${dir}/color.png`, [128, 128, 128, 255]);
    layerInto(nrm, layer, `/data/${dir}/normal.png`, [128, 128, 255, 255]);
    layerInto(rgh, layer, `/data/${dir}/rough.png`, [220, 220, 220, 255]);
  }

  // Per-class shader table: x = layer A (-1 = tint only), y = layer B
  // (-1 = none), z = B amount, w = flags (bit0 water, bit1 B-mode "top").
  const mats = new Float32Array(MAXC * 4).fill(-1);
  const tints = new Float32Array(MAXC * 3);
  for (const c of classes) {
    if (c.id >= MAXC) continue;
    const m0 = c.materials[0];
    const m1 = c.materials[1];
    mats[c.id * 4] = m0 ? layerOf.get(m0.dir) : -1;
    mats[c.id * 4 + 1] = m1 ? layerOf.get(m1.dir) : -1;
    mats[c.id * 4 + 2] = m1 ? m1.amount : 0;
    mats[c.id * 4 + 3] = (c.water ? 1 : 0) + (m1 && m1.mode === 'top' ? 2 : 0);
    tints.set(hexToRgb(c.avg_color || c.color), c.id * 3);
  }
  classState = { alb, nrm, rgh, mats, tints };
  for (const mat of registry) applyClassUniforms(mat);
}

function applyClassUniforms(mat) {
  const sh = mat.userData.shader;
  if (!sh || !classState) return;
  sh.uniforms.uAlb.value = classState.alb;
  sh.uniforms.uNrm.value = classState.nrm;
  sh.uniforms.uRgh.value = classState.rgh;
  sh.uniforms.uClassMat.value = classState.mats;
  sh.uniforms.uClassTint.value = classState.tints;
  sh.uniforms.uClassReady.value = 1.0;
}

const NOISE_GLSL = /* glsl */ `
  float thash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
  vec2 thash2(vec2 p) {
    return fract(sin(vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)))) * 43758.5453);
  }
  float tnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(thash(i), thash(i + vec2(1.0, 0.0)), f.x),
               mix(thash(i + vec2(0.0, 1.0)), thash(i + vec2(1.0, 1.0)), f.x), f.y);
  }
  // Triangular grid for stochastic tiling: 3 nearest cells + blend weights.
  void triGrid(vec2 uv, out vec3 w, out vec2 v1, out vec2 v2, out vec2 v3) {
    const mat2 skew = mat2(1.0, -0.57735027, 0.0, 1.15470054);
    vec2 s = skew * (uv * 0.35);
    vec2 base = floor(s);
    vec3 t = vec3(fract(s), 0.0);
    t.z = 1.0 - t.x - t.y;
    if (t.z > 0.0) {
      w = vec3(t.z, t.y, t.x);
      v1 = base; v2 = base + vec2(0.0, 1.0); v3 = base + vec2(1.0, 0.0);
    } else {
      w = vec3(-t.z, 1.0 - t.y, 1.0 - t.x);
      v1 = base + vec2(1.0, 1.0); v2 = base + vec2(1.0, 0.0); v3 = base + vec2(0.0, 1.0);
    }
    w = w * w / dot(w * w, vec3(1.0)); // sharpen: mostly one tap dominates
  }
  mat2 trot(float a) {
    float c = cos(a), s = sin(a);
    return mat2(c, -s, s, c);
  }
  // Sample with a random rotation AND offset per grid cell — periodicity
  // is fully broken, so the repeat can never be spotted at any distance.
  vec4 stex(sampler2D t, vec2 uv, vec3 w, vec2 v1, vec2 v2, vec2 v3, vec2 dx, vec2 dy) {
    mat2 R1 = trot(thash(v1) * 6.2832);
    mat2 R2 = trot(thash(v2) * 6.2832);
    mat2 R3 = trot(thash(v3) * 6.2832);
    return textureGrad(t, R1 * uv + thash2(v1), R1 * dx, R1 * dy) * w.x
         + textureGrad(t, R2 * uv + thash2(v2), R2 * dx, R2 * dy) * w.y
         + textureGrad(t, R3 * uv + thash2(v3), R3 * dx, R3 * dy) * w.z;
  }
  vec3 stexN(sampler2D t, vec2 uv, vec3 w, vec2 v1, vec2 v2, vec2 v3, vec2 dx, vec2 dy) {
    float a1 = thash(v1) * 6.2832;
    float a2 = thash(v2) * 6.2832;
    float a3 = thash(v3) * 6.2832;
    mat2 R1 = trot(a1), R2 = trot(a2), R3 = trot(a3);
    vec3 n1 = textureGrad(t, R1 * uv + thash2(v1), R1 * dx, R1 * dy).rgb * 2.0 - 1.0;
    vec3 n2 = textureGrad(t, R2 * uv + thash2(v2), R2 * dx, R2 * dy).rgb * 2.0 - 1.0;
    vec3 n3 = textureGrad(t, R3 * uv + thash2(v3), R3 * dx, R3 * dy).rgb * 2.0 - 1.0;
    n1.xy = trot(-a1) * n1.xy;
    n2.xy = trot(-a2) * n2.xy;
    n3.xy = trot(-a3) * n3.xy;
    return n1 * w.x + n2 * w.y + n3 * w.z;
  }
  // Array-layer variants for the class materials.
  vec4 stexL(mediump sampler2DArray t, float l, vec2 uv, vec3 w, vec2 v1, vec2 v2, vec2 v3, vec2 dx, vec2 dy) {
    mat2 R1 = trot(thash(v1) * 6.2832);
    mat2 R2 = trot(thash(v2) * 6.2832);
    mat2 R3 = trot(thash(v3) * 6.2832);
    return textureGrad(t, vec3(R1 * uv + thash2(v1), l), R1 * dx, R1 * dy) * w.x
         + textureGrad(t, vec3(R2 * uv + thash2(v2), l), R2 * dx, R2 * dy) * w.y
         + textureGrad(t, vec3(R3 * uv + thash2(v3), l), R3 * dx, R3 * dy) * w.z;
  }
  vec3 stexNL(mediump sampler2DArray t, float l, vec2 uv, vec3 w, vec2 v1, vec2 v2, vec2 v3, vec2 dx, vec2 dy) {
    float a1 = thash(v1) * 6.2832;
    float a2 = thash(v2) * 6.2832;
    float a3 = thash(v3) * 6.2832;
    mat2 R1 = trot(a1), R2 = trot(a2), R3 = trot(a3);
    vec3 n1 = textureGrad(t, vec3(R1 * uv + thash2(v1), l), R1 * dx, R1 * dy).rgb * 2.0 - 1.0;
    vec3 n2 = textureGrad(t, vec3(R2 * uv + thash2(v2), l), R2 * dx, R2 * dy).rgb * 2.0 - 1.0;
    vec3 n3 = textureGrad(t, vec3(R3 * uv + thash2(v3), l), R3 * dx, R3 * dy).rgb * 2.0 - 1.0;
    n1.xy = trot(-a1) * n1.xy;
    n2.xy = trot(-a2) * n2.xy;
    n3.xy = trot(-a3) * n3.xy;
    return n1 * w.x + n2 * w.y + n3 * w.z;
  }
  // Triplanar sample for steep faces — no stretching on cliffs.
  vec4 tri(sampler2D t, vec2 ux, vec2 uy, vec2 uz, vec3 w) {
    return texture(t, ux) * w.x + texture(t, uy) * w.y + texture(t, uz) * w.z;
  }
`;

// options.classIdx/classW: this tile's top-4 class textures from
// class.bin (idx MUST be NEAREST-filtered). Without them the material
// renders the procedural fallback.
export function createTerrainMaterial(options = {}) {
  const mat = new THREE.MeshStandardMaterial({ roughness: 1.0, metalness: 0.0 });
  mat.defines = { USE_UV: '' }; // the uv attribute feeds the class lookup
  const tex = loadTextures();
  const neutral = placeholder('c');
  const useClass = !!(options.classIdx && options.classW);

  mat.onBeforeCompile = (shader) => {
    for (const key of Object.keys(MATERIALS)) {
      shader.uniforms[key] = { value: tex[key] };
    }
    shader.uniforms.uClassIdx = { value: options.classIdx || neutral };
    shader.uniforms.uClassW = { value: options.classW || neutral };
    shader.uniforms.uUseClass = { value: useClass ? 1.0 : 0.0 };
    shader.uniforms.uClassReady = { value: classState ? 1.0 : 0.0 };
    shader.uniforms.uAlb = { value: classState ? classState.alb : makeArray(0, true) };
    shader.uniforms.uNrm = { value: classState ? classState.nrm : makeArray(0, false) };
    shader.uniforms.uRgh = { value: classState ? classState.rgh : makeArray(0, false) };
    shader.uniforms.uClassMat = {
      value: classState ? classState.mats : new Float32Array(MAXC * 4).fill(-1),
    };
    shader.uniforms.uClassTint = {
      value: classState ? classState.tints : new Float32Array(MAXC * 3),
    };
    mat.userData.shader = shader;

    shader.vertexShader = shader.vertexShader
      .replace(
        '#include <common>',
        `#include <common>
         varying vec3 vWPos;
         varying vec3 vWNormal;
         varying float vWater;
         varying vec2 vTileUv;`,
      )
      .replace(
        '#include <begin_vertex>',
        `#include <begin_vertex>
         vTileUv = uv;
         vWater = step(abs(transformed.y), 0.005);
         transformed.y -= ${WATER_DROP.toFixed(2)} * vWater;`,
      )
      .replace(
        '#include <fog_vertex>',
        `#include <fog_vertex>
         vec4 tWp = vec4(transformed, 1.0);
         vec3 tWn = objectNormal;
         #ifdef USE_BATCHING
           tWp = batchingMatrix * tWp;
           tWn = mat3(batchingMatrix) * tWn;
         #endif
         vWPos = (modelMatrix * tWp).xyz;
         vWNormal = normalize(mat3(modelMatrix) * tWn);`,
      );

    shader.fragmentShader = shader.fragmentShader
      .replace(
        '#include <common>',
        `#include <common>
         varying vec3 vWPos;
         varying vec3 vWNormal;
         varying float vWater;
         varying vec2 vTileUv;
         uniform sampler2D uGAc, uGAn, uGAr, uGAd;
         uniform sampler2D uGBc, uGBn, uGBr, uGBd;
         uniform sampler2D uRc, uRn, uRr, uRd;
         uniform mediump sampler2DArray uAlb;
         uniform mediump sampler2DArray uNrm;
         uniform mediump sampler2DArray uRgh;
         uniform sampler2D uClassIdx, uClassW;
         uniform vec4 uClassMat[${MAXC}];
         uniform vec3 uClassTint[${MAXC}];
         uniform float uUseClass, uClassReady;
         ${NOISE_GLSL}`,
      )
      .replace(
        '#include <map_fragment>',
        `#include <map_fragment>
         float tWater = clamp(vWater, 0.0, 1.0);
         float tFade = exp(-length(vViewPosition) / 900.0);
         vec3 tFacetW = normalize(cross(dFdx(vWPos), dFdy(vWPos)));
         tFacetW *= sign(tFacetW.y + 1e-6);
         #ifndef USE_MAP
           float tH = vWPos.y;
           float tSlope = degrees(acos(clamp(tFacetW.y, 0.0, 1.0)));

           // Grounds repeat every 5 m, rock every 10 m; rock is triplanar
           // so cliff faces get an unstretched side projection.
           vec2 uvG = vWPos.xz * 0.2;
           vec3 wG; vec2 g1, g2, g3; triGrid(uvG, wG, g1, g2, g3);
           vec2 gdx = dFdx(uvG), gdy = dFdy(uvG);

           vec3 twr = pow(abs(tFacetW), vec3(4.0));
           twr /= (twr.x + twr.y + twr.z);
           vec2 uRx = vWPos.zy * 0.1;
           vec2 uRy = vWPos.xz * 0.1;
           vec2 uRz = vWPos.xy * 0.1;

           float dA = texture(uGAd, uvG).r;
           float dB = texture(uGBd, uvG).r;
           float dR = texture(uRd, uRy).r;

           // Procedural fallback: two grounds by noise, rock on slopes.
           float gmix = tnoise(vWPos.xz * 0.02) * 0.65 + tnoise(vWPos.xz * 0.0045) * 0.35;
           float gm = clamp((gmix - 0.5) * 2.2 + 0.5 + (dB - dA) * 0.9, 0.0, 1.0);
           float rBase = smoothstep(27.0, 42.0, tSlope);
           float rw = clamp(rBase + (dR - 0.5) * rBase * (1.0 - rBase) * 3.2, 0.0, 1.0);

           vec3 col = mix(
             mix(stex(uGAc, uvG, wG, g1, g2, g3, gdx, gdy).rgb,
                 stex(uGBc, uvG, wG, g1, g2, g3, gdx, gdy).rgb, gm),
             tri(uRc, uRx, uRy, uRz, twr).rgb, rw);
           vec3 tNormal = mix(
             mix(stexN(uGAn, uvG, wG, g1, g2, g3, gdx, gdy),
                 stexN(uGBn, uvG, wG, g1, g2, g3, gdx, gdy), gm),
             tri(uRn, uRx, uRy, uRz, twr).rgb * 2.0 - 1.0, rw);
           float tRough = mix(
             mix(stex(uGAr, uvG, wG, g1, g2, g3, gdx, gdy).r,
                 stex(uGBr, uvG, wG, g1, g2, g3, gdx, gdy).r, gm),
             tri(uRr, uRx, uRy, uRz, twr).r, rw);

           // Class-driven weights: top-4 (index, weight) baked per tile.
           if (uUseClass > 0.5 && uClassReady > 0.5) {
             vec4 ci = texture(uClassIdx, vTileUv) * 255.0;
             vec4 cw = texture(uClassW, vTileUv);
             float wsum = max(cw.r + cw.g + cw.b + cw.a, 1e-4);
             vec3 accC = vec3(0.0);
             vec3 accN = vec3(0.0);
             float accR = 0.0;
             float waterW = 0.0;
             for (int k = 0; k < 4; k++) {
               float wk = cw[k] / wsum;
               if (wk < 0.004) continue;
               int id = int(ci[k] + 0.5);
               if (id >= ${MAXC}) continue;
               vec4 cm = uClassMat[id];
               vec3 ck; vec3 nk; float rk;
               if (cm.x < 0.0) {
                 ck = uClassTint[id];
                 nk = vec3(0.0, 0.0, 1.0);
                 rk = 0.75;
               } else {
                 ck = stexL(uAlb, cm.x, uvG, wG, g1, g2, g3, gdx, gdy).rgb;
                 nk = stexNL(uNrm, cm.x, uvG, wG, g1, g2, g3, gdx, gdy);
                 rk = stexL(uRgh, cm.x, uvG, wG, g1, g2, g3, gdx, gdy).r;
                 if (cm.y >= 0.0) {
                   // Second material in the class: noise mix or on-top
                   // patches, steered by its amount.
                   float mB = mod(cm.w, 4.0) >= 2.0
                     ? step(1.0 - clamp(cm.z, 0.0, 1.0), tnoise(vWPos.xz * 0.031))
                     : clamp(cm.z, 0.0, 1.0) * (0.35 + 0.65 * tnoise(vWPos.xz * 0.011));
                   ck = mix(ck, stexL(uAlb, cm.y, uvG, wG, g1, g2, g3, gdx, gdy).rgb, mB);
                   nk = mix(nk, stexNL(uNrm, cm.y, uvG, wG, g1, g2, g3, gdx, gdy), mB);
                   rk = mix(rk, stexL(uRgh, cm.y, uvG, wG, g1, g2, g3, gdx, gdy).r, mB);
                 }
               }
               if (mod(cm.w, 2.0) >= 1.0) waterW += wk;
               accC += ck * wk;
               accN += nk * wk;
               accR += rk * wk;
             }
             col = accC;
             tNormal = accN;
             tRough = accR;
             tWater = max(tWater, waterW);
           }

           // Large-scale tint so distant terrain never looks uniform.
           float macro = tnoise(vWPos.xz * 0.0012) * 0.6 + tnoise(vWPos.xz * 0.009) * 0.4;
           col *= 0.86 + 0.28 * macro;

           vec3 tWaterCol = mix(vec3(0.14, 0.36, 0.42), vec3(0.05, 0.21, 0.35), smoothstep(0.3, 1.0, tWater));
           col = mix(col, tWaterCol, smoothstep(0.15, 0.75, tWater));
           diffuseColor.rgb = col;
         #endif`,
      )
      .replace(
        '#include <roughnessmap_fragment>',
        `#include <roughnessmap_fragment>
         #ifndef USE_MAP
           roughnessFactor = clamp(mix(tRough, 0.15, tWater), 0.05, 1.0);
         #else
           roughnessFactor = mix(roughnessFactor, 0.25, tWater);
         #endif`,
      )
      .replace(
        '#include <normal_fragment_begin>',
        `#include <normal_fragment_begin>
         #ifndef USE_MAP
           vec3 tNg = normalize(mix(normalize(vWNormal), tFacetW, 0.35 + 0.35 * tFade));
           vec3 tT = normalize(vec3(1.0, 0.0, 0.0) - tNg * tNg.x);
           vec3 tB = cross(tNg, tT);
           vec3 tMapN = tNormal;
           tMapN.xy *= (0.15 + 0.85 * tFade) * (1.0 - tWater);
           vec3 tWorldN = normalize(tT * tMapN.x + tB * tMapN.y + tNg * max(tMapN.z, 0.2));
           tWorldN = mix(tWorldN, vec3(0.0, 1.0, 0.0), tWater);
           normal = normalize((viewMatrix * vec4(tWorldN, 0.0)).xyz);
         #endif`,
      );

    if (classState) applyClassUniforms(mat);
  };

  registry.add(mat);
  const origDispose = mat.dispose.bind(mat);
  mat.dispose = () => {
    registry.delete(mat);
    origDispose();
  };
  return mat;
}
