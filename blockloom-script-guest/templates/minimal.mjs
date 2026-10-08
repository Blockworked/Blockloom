// Minimal JavaScript/TypeScript guest: the same contract as minimal.rs, .c
// and .py. Built against the frozen world with jco (componentize-js):
//
//   npm i @bytecodealliance/jco
//   npx jco componentize minimal.mjs --wit wit --world-name script --out minimal.wasm
//
// then put minimal.wasm under assets/scripts and add it as the actor's script.
// TypeScript compiles to this shape first (tsc/esbuild), then componentizes.
import { act } from 'blockloom:script/acts@0.2.0';
import { read } from 'blockloom:script/sensors@0.2.0';

// abi.rs verb numbers (blockloom-core/src/script/abi.rs).
const READ_POSITION = 1;
const ACT_CHANGE_POSITION = 3;
const ACT_SAY = 10;

export const entry = {
  start() { act(ACT_SAY, 'hello from wasm', '', '', []); },
  tick(dt) {
    read(READ_POSITION, '', '', 0);
    act(ACT_CHANGE_POSITION, '', '', '', new Float64Array([0, 1]));
  },
  frame(dt) {},
  ui(dt) {},
  stop() {},
  destroy() {},
  onEvent(kind, n0, n1, n2, n3) { act(ACT_SAY, 'event heard', '', '', []); },
};
