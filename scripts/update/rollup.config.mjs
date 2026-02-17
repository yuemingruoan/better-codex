// Rollup config for bundling scripts/update.ts to update.js with all deps
import typescript from "@rollup/plugin-typescript";
import { nodeResolve } from "@rollup/plugin-node-resolve";
import commonjs from "@rollup/plugin-commonjs";
import json from "@rollup/plugin-json";
import terser from "@rollup/plugin-terser";

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
    typescript({
      tsconfig: "./tsconfig.json",
      outDir: "scripts",
      filterRoot: "scripts",
      sourceMap: false,
    }),
    nodeResolve({ preferBuiltins: true }),
    commonjs(),
    json(),
    terser(),
  ],
  external: [], // bundle all except node builtins
};
