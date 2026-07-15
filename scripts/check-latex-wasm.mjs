#!/usr/bin/env node

import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const [packageArgument, bundleArgument, sourceArgument, outputArgument] =
	process.argv.slice(2);
if (!packageArgument || !bundleArgument || !sourceArgument || !outputArgument) {
	throw new Error(
		"usage: check-latex-wasm.mjs PACKAGE-DIR BUNDLE-DIR SOURCE.tex OUTPUT-DIR",
	);
}

const packageDirectory = path.resolve(packageArgument);
const bundleDirectory = path.resolve(bundleArgument);
const sourcePath = path.resolve(sourceArgument);
const outputDirectory = path.resolve(outputArgument);
const bindings = await import(
	pathToFileURL(path.join(packageDirectory, "umber_wasm.js"))
);
const { compile } = await import(
	pathToFileURL(path.join(packageDirectory, "compile.js"))
);
const { HttpManifestResolver } = await import(
	pathToFileURL(path.join(packageDirectory, "manifest-resolver.js"))
);

const wasm = await readFile(path.join(packageDirectory, "umber_wasm_bg.wasm"));
await bindings.default({ module_or_path: wasm });
const manifest = JSON.parse(
	await readFile(path.join(bundleDirectory, "manifest.json"), "utf8"),
);
manifest.objectsBaseUrl = "https://umber.invalid/objects/";
const fetchObject = async (url) => {
	const parsed = new URL(url);
	assert.equal(parsed.origin, "https://umber.invalid");
	const name = parsed.pathname.slice("/objects/".length);
	const bytes = await readFile(path.join(bundleDirectory, "objects", name));
	return new Response(bytes, {
		headers: { "content-length": String(bytes.byteLength) },
	});
};
const resolver = new HttpManifestResolver(manifest, {
	fetch: fetchObject,
	crypto: webcrypto,
	maxFiles: 512,
	maxBytes: 64 * 1024 * 1024,
});
const format = await resolver.resolveFormat("latex", {
	engineVersion: bindings.packageVersion(),
	formatSchema: bindings.formatSchemaVersion(),
});

const files = new Map([
	["document.tex", new Uint8Array(await readFile(sourcePath))],
	["document.aux", new Uint8Array()],
	["document.toc", new Uint8Array()],
]);
let output;
for (let pass = 1; pass <= 3; pass += 1) {
	output = await compile(
		{
			mainPath: "document.tex",
			jobName: "document",
			format,
			clock: { year: 2026, month: 7, day: 14, minutes: 22 * 60 },
		},
		files,
		resolver,
		undefined,
		bindings,
	);
	for (const file of output.files) {
		const relative = file.path.startsWith("/job/")
			? file.path.slice("/job/".length)
			: file.path;
		files.set(relative, file.bytes.slice());
	}
}

await mkdir(outputDirectory, { recursive: true });
await writeFile(path.join(outputDirectory, "document.dvi"), output.dvi);
for (const file of output.files) {
	const relative = file.path.startsWith("/job/")
		? file.path.slice("/job/".length)
		: file.path;
	await writeFile(path.join(outputDirectory, relative), file.bytes);
}
process.stdout.write(
	`LaTeX WASM parity output: dvi=${output.dvi.byteLength} files=${output.files.length}\n`,
);
