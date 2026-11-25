// Minimal TypeScript guest: the same contract as minimal.rs, .c and .py.
// Built by bundling to JavaScript (esbuild) and then componentizing with jco
// against the frozen world; `just`/Play does both for you.
import { act } from 'blockloom:script/acts@0.2.0';
import { read } from 'blockloom:script/sensors@0.2.0';

// abi.rs verb numbers (blockloom-core/src/script/abi.rs).
const READ_POSITION = 1;
const ACT_CHANGE_POSITION = 3;
const ACT_SAY = 10;

export const entry = {
  start(): void {
    act(ACT_SAY, 'hello from wasm', '', '', []);
  },
  tick(dt: number): void {
    read(READ_POSITION, '', '', 0);
    act(ACT_CHANGE_POSITION, '', '', '', new Float64Array([0, 1]));
  },
  frame(dt: number): void {},
  ui(dt: number): void {},
  stop(): void {},
  destroy(): void {},
  onEvent(kind: number, n0: number, n1: number, n2: number, n3: number): void {
    act(ACT_SAY, 'event heard', '', '', []);
  },
};
