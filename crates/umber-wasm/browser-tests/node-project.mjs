import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const packageDirectory = path.resolve(process.argv[2]);
const bindings = await import(
	pathToFileURL(path.join(packageDirectory, "umber_wasm.js"))
);
const wasm = await readFile(path.join(packageDirectory, "umber_wasm_bg.wasm"));
bindings.initSync({ module: wasm });
const { compile } = await import(
	pathToFileURL(path.join(packageDirectory, "compile.js"))
);
const source = new TextEncoder().encode(
	'\\immediate\\openout1=main.bcf\\immediate\\write1{<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex"><bcf:section number="0"></bcf:section></bcf:controlfile>}\\immediate\\closeout1\\shipout\\hbox{N}\\end',
);
const output = await compile(
	{
		mainPath: "/job/main.tex",
		bibliography: {
			controlPath: "/job/main.bcf",
			outputs: [{ path: "/job/main.bbl", format: "bbl" }],
		},
	},
	new Map([["/job/main.tex", source]]),
	{
		async resolve() {
			return [];
		},
	},
	undefined,
	bindings,
);
assert(output.passes >= 2);
assert(output.tex.dvi.byteLength > 0);
assert(output.generatedFiles.some((file) => file.path === "/job/main.bbl"));
