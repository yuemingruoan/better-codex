// Rollup config for bundling scripts/update/update.ts to update.js with all deps
import path from "node:path";
import typescript from "@rollup/plugin-typescript";
import { nodeResolve } from "@rollup/plugin-node-resolve";
import commonjs from "@rollup/plugin-commonjs";
import json from "@rollup/plugin-json";
import terser from "@rollup/plugin-terser";

const inputFile = path.resolve(import.meta.dirname, "update.ts");
const outputFile = path.resolve(import.meta.dirname, "better-codex-update.js");
const tsconfigFile = path.resolve(import.meta.dirname, "tsconfig.json");

/** @type {import('rollup').RollupOptions} */
export default {
  input: inputFile,
  output: {
    file: outputFile,
    format: "cjs",
    sourcemap: false,
    banner: "#!/usr/bin/env node",
  },
  plugins: [
    typescript({
      tsconfig: tsconfigFile,
      outDir: import.meta.dirname,
      filterRoot: import.meta.dirname,
      sourceMap: false,
    }),
    nodeResolve({ preferBuiltins: true }),
    commonjs(),
    json(),
    terser(),
  ],
  external: [], // bundle all except node builtins
};
