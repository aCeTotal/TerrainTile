// Procedural terrain material shared by the near and far layers when no
// orthophoto exists: per-fragment height/slope color ramp (same thresholds
// as the DTM mask classifier), blue water at sea level, faceted normal
// detail up close, and full shadow support via MeshStandardMaterial.

import * as THREE from 'three';

export function createTerrainMaterial() {
  const mat = new THREE.MeshStandardMaterial({ roughness: 1.0, metalness: 0.0 });

  mat.onBeforeCompile = (shader) => {
    shader.vertexShader = shader.vertexShader
      .replace(
        '#include <common>',
        `#include <common>
         varying vec3 vWPos;
         varying vec3 vWNormal;`,
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
         float thash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }`,
      )
      .replace(
        '#include <map_fragment>',
        `#include <map_fragment>
         float tH = vWPos.y;
         vec3 tWn = normalize(vWNormal);
         float tSlope = degrees(acos(clamp(tWn.y, 0.0, 1.0)));
         // Water: flat and at the sea-level fill height (nodata = 0 m).
         float tWater = (1.0 - smoothstep(0.05, 0.5, tH)) * smoothstep(0.9962, 0.9995, tWn.y);
         #ifndef USE_MAP
           float tFade = exp(-length(vViewPosition) / 1200.0);
           float tn = thash(floor(vWPos.xz));
           float tn2 = thash(floor(vWPos.xz * 0.083));
           vec3 tGrass = mix(vec3(0.30, 0.44, 0.19), vec3(0.42, 0.47, 0.23), tn2);
           tGrass = mix(tGrass, vec3(0.29, 0.35, 0.19), smoothstep(300.0, 900.0, tH));
           vec3 tDirt = mix(vec3(0.40, 0.33, 0.23), vec3(0.32, 0.26, 0.18), tn2);
           vec3 tRock = mix(vec3(0.41, 0.40, 0.39), vec3(0.53, 0.52, 0.50), tn);
           vec3 tSnow = vec3(0.90, 0.92, 0.96);
           vec3 tc = tGrass;
           tc = mix(tc, tDirt, smoothstep(18.0, 32.0, tSlope));
           tc = mix(tc, tRock, smoothstep(35.0, 50.0, tSlope));
           tc = mix(tc, tSnow, smoothstep(1000.0, 1250.0, tH) * (1.0 - smoothstep(38.0, 55.0, tSlope)));
           tc *= 1.0 + (tn - 0.5) * 0.20 * tFade;
           tc = mix(tc, mix(vec3(0.05, 0.20, 0.35), vec3(0.10, 0.31, 0.44), tn2), tWater);
           diffuseColor.rgb = tc;
         #endif`,
      )
      .replace(
        '#include <roughnessmap_fragment>',
        `#include <roughnessmap_fragment>
         roughnessFactor = mix(roughnessFactor, 0.25, tWater);`,
      )
      .replace(
        '#include <normal_fragment_begin>',
        `#include <normal_fragment_begin>
         #ifndef USE_MAP
           vec3 tFdx = dFdx(vViewPosition);
           vec3 tFdy = dFdy(vViewPosition);
           vec3 tFacet = normalize(cross(tFdx, tFdy));
           normal = normalize(mix(normal, tFacet, 0.45 * tFade * (1.0 - tWater)));
         #endif`,
      );
  };

  return mat;
}
