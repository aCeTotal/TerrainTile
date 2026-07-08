// Cheap procedural terrain material for everything outside the close-up
// zone: slope/height color ramp, no texture taps — a few ALU ops per
// pixel. Colors are tuned to roughly match the averages of the PBR
// texture set in terrain-material.js so the handover is not jarring.
// Shares the water plane convention (fill plane sunk WATER_DROP).

import * as THREE from 'three';

export const WATER_DROP = 0.7; // meters the 0 m fill plane is lowered

export function createSimpleTerrainMaterial() {
  const mat = new THREE.MeshStandardMaterial({ roughness: 1.0, metalness: 0.0 });

  mat.onBeforeCompile = (shader) => {
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
         float thash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
         float tnoise(vec2 p) {
           vec2 i = floor(p);
           vec2 f = fract(p);
           f = f * f * (3.0 - 2.0 * f);
           return mix(mix(thash(i), thash(i + vec2(1.0, 0.0)), f.x),
                      mix(thash(i + vec2(0.0, 1.0)), thash(i + vec2(1.0, 1.0)), f.x), f.y);
         }`,
      )
      .replace(
        '#include <map_fragment>',
        `#include <map_fragment>
         float tWater = clamp(vWater, 0.0, 1.0);
         float tFade = exp(-length(vViewPosition) / 900.0);
         vec3 tFacetW = normalize(cross(dFdx(vWPos), dFdy(vWPos)));
         tFacetW *= sign(tFacetW.y + 1e-6);
         float tH = vWPos.y;
         float tSlope = degrees(acos(clamp(tFacetW.y, 0.0, 1.0)));
         float tn2 = tnoise(vWPos.xz * 0.02) * 0.65 + tnoise(vWPos.xz * 0.0045) * 0.35;
         vec3 tGrass = mix(vec3(0.30, 0.40, 0.18), vec3(0.41, 0.43, 0.22), tn2);
         tGrass = mix(tGrass, vec3(0.29, 0.34, 0.19), smoothstep(300.0, 900.0, tH));
         vec3 tDirt = vec3(0.38, 0.32, 0.23);
         vec3 tRock = vec3(0.46, 0.45, 0.43);
         vec3 tc = tGrass;
         tc = mix(tc, tDirt, smoothstep(18.0, 32.0, tSlope));
         tc = mix(tc, tRock, smoothstep(30.0, 45.0, tSlope));
         tc = mix(tc, vec3(0.90, 0.92, 0.96),
                  smoothstep(1000.0, 1250.0, tH) * (1.0 - smoothstep(38.0, 55.0, tSlope)));
         float macro = tnoise(vWPos.xz * 0.0012) * 0.6 + tnoise(vWPos.xz * 0.009) * 0.4;
         tc *= 0.86 + 0.28 * macro;
         vec3 tWaterCol = mix(vec3(0.14, 0.36, 0.42), vec3(0.05, 0.21, 0.35), smoothstep(0.3, 1.0, tWater));
         tc = mix(tc, tWaterCol, smoothstep(0.15, 0.75, tWater));
         diffuseColor.rgb = tc;`,
      )
      .replace(
        '#include <roughnessmap_fragment>',
        `#include <roughnessmap_fragment>
         roughnessFactor = mix(roughnessFactor, 0.15, tWater);`,
      )
      .replace(
        '#include <normal_fragment_begin>',
        `#include <normal_fragment_begin>
         vec3 tNg = normalize(mix(normalize(vWNormal), tFacetW, 0.4 * tFade));
         tNg = mix(tNg, vec3(0.0, 1.0, 0.0), tWater);
         normal = normalize((viewMatrix * vec4(tNg, 0.0)).xyz);`,
      );
  };

  return mat;
}
