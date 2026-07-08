// PBR terrain material shared by the near and far layers when no
// orthophoto exists. Three ambientCG material sets (served from
// /materials/): two grounds blended by noise on flat terrain, rock on
// steep slopes. Displacement maps sharpen every transition, normal and
// roughness maps carry the close-up detail, and all sampling is
// stochastic (per-cell random offsets over a triangular grid) so the
// 4-8 m repeats never show, near or far.
//
// Water is the nodata fill plane at exactly 0 m: sunk WATER_DROP in the
// vertex stage so tidal-zone terrain stands above the surface.

import * as THREE from 'three';

const WATER_DROP = 0.7; // meters the fill plane is lowered in the viewer

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
  // Same, for normal maps: the sampled tangent normal is rotated back so
  // lighting stays physically consistent across cells.
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
  // Triplanar sample for steep faces — no stretching on cliffs.
  vec4 tri(sampler2D t, vec2 ux, vec2 uy, vec2 uz, vec3 w) {
    return texture(t, ux) * w.x + texture(t, uy) * w.y + texture(t, uz) * w.z;
  }
`;

export function createTerrainMaterial() {
  const mat = new THREE.MeshStandardMaterial({ roughness: 1.0, metalness: 0.0 });
  const tex = loadTextures();

  mat.onBeforeCompile = (shader) => {
    for (const key of Object.keys(MATERIALS)) {
      shader.uniforms[key] = { value: tex[key] };
    }

    shader.vertexShader = shader.vertexShader
      .replace(
        '#include <common>',
        `#include <common>
         varying vec3 vWPos;
         varying vec3 vWNormal;
         varying float vWater;`,
      )
      .replace(
        '#include <begin_vertex>',
        `#include <begin_vertex>
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
         uniform sampler2D uGAc, uGAn, uGAr, uGAd;
         uniform sampler2D uGBc, uGBn, uGBr, uGBd;
         uniform sampler2D uRc, uRn, uRr, uRd;
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

           // Player-scale physical sizes: grounds repeat every 5 m
           // (2K px / 5 m = 2.4 mm per pixel at boot height), rock every
           // 10 m. Grounds are planar (they live on flat terrain, no
           // stretch); rock is TRIPLANAR so cliff faces get an unstretched
           // projection from the side.
           vec2 uvG = vWPos.xz * 0.2;
           vec3 wG; vec2 g1, g2, g3; triGrid(uvG, wG, g1, g2, g3);
           vec2 gdx = dFdx(uvG), gdy = dFdy(uvG);

           vec3 twr = pow(abs(tFacetW), vec3(4.0));
           twr /= (twr.x + twr.y + twr.z);
           vec2 uRx = vWPos.zy * 0.1;
           vec2 uRy = vWPos.xz * 0.1;
           vec2 uRz = vWPos.xy * 0.1;

           // Displacement only steers blend weights — plain taps suffice.
           float dA = texture(uGAd, uvG).r;
           float dB = texture(uGBd, uvG).r;
           float dR = texture(uRd, uRy).r;

           // Ground A/B: noise patches, displacement sharpens the border.
           float gmix = tnoise(vWPos.xz * 0.02) * 0.65 + tnoise(vWPos.xz * 0.0045) * 0.35;
           float gm = clamp((gmix - 0.5) * 2.2 + 0.5 + (dB - dA) * 0.9, 0.0, 1.0);

           // Rock on steep slopes; displacement lets it break through in
           // patches along the transition instead of a straight line.
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

           // Snow above the tree line, procedural.
           float sw = smoothstep(1000.0, 1250.0, tH) * (1.0 - smoothstep(38.0, 55.0, tSlope));
           col = mix(col, vec3(0.90, 0.92, 0.96), sw);
           tNormal = mix(tNormal, vec3(0.0, 0.0, 1.0), sw * 0.8);
           tRough = mix(tRough, 0.55, sw);

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
  };

  return mat;
}
