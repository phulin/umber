#!/usr/bin/env node

import path from "node:path";
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const packageDirectory = path.resolve(
	process.argv[2] ?? "target/umber-wasm-package",
);
const bindings = await import(
	pathToFileURL(path.join(packageDirectory, "umber_wasm.js"))
);
const wasmBytes = await readFile(path.join(packageDirectory, "umber_wasm_bg.wasm"));
const exports = bindings.initSync({ module: wasmBytes });
const memoryBytes = () => exports.memory.buffer.byteLength;
const source = `${Array.from(
	{ length: 4_000 },
	(_, index) =>
		`\\hbox{\\vrule width${(index % 7) + 1}pt height10pt}\\par`,
).join("\n")}\n\\vfill\\eject\\end\n`;

globalThis.gc?.();
const before = memoryBytes();
const session = new bindings.EditorSession({
	mainPath: "main.tex",
	engine: "tex82",
	outputs: ["dvi"],
});
session.addUserFile("main.tex", new TextEncoder().encode(source));
const created = memoryBytes();
const result = session.advance();
if (result.kind !== "stable") {
	throw new Error(`self-contained editor workload did not stabilize: ${result.kind}`);
}
const compiled = memoryBytes();
session.dispose();
session.free();
globalThis.gc?.();
const disposed = memoryBytes();

console.log(
	JSON.stringify({
		workload: "self-contained-4000-rule-paragraphs",
		wasmPageBytes: 65_536,
		before,
		created,
		compiled,
		disposed,
		compileGrowthBytes: compiled - before,
		compileGrowthPages: (compiled - before) / 65_536,
	}),
);
