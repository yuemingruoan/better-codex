// Rollup config for bundling scripts/update.ts to update.js with all deps
import typescript from "@rollup/plugin-typescript";
import { nodeResolve } from "@rollup/plugin-node-resolve";
import commonjs from "@rollup/plugin-commonjs";
import json from "@rollup/plugin-json";

/** @type {import('rollup').RollupOptions} */
export default {
  input: "scripts/update.ts",
  output: {
    file: "scripts/update.js",
    format: "cjs",
    sourcemap: false,
    banner: "#!/usr/bin/env node",
  },
  plugins: [
    nodeResolve({ preferBuiltins: true }),
    commonjs(),
    json(),
    typescript({ tsconfig: "./tsconfig.json", sourceMap: false }),
  ],
  external: [], // bundle all except node builtins
};
