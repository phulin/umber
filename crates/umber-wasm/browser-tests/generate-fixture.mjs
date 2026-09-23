import { execFileSync } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { deterministicAhash64Hex } from "../js/manifest-resolver.js";

const encoder = new TextEncoder();

// The production publisher writes the packed shards. This small manifest only
// supplies deterministic payloads and their declared identities.
export async function generateFixture(directory, objectsBaseUrl, publisher) {
	const objectsDirectory = path.join(directory, "objects");
	await mkdir(objectsDirectory, { recursive: true });
	const files = {};
	for (const [name, source, dependencies] of [
		[
			"probe.tex",
			"\\message{PACKED-CATALOG-RESOURCE}\\relax",
			["tex:hint.tex"],
		],
		["hint.tex", "\\relax", []],
	]) {
		const bytes = encoder.encode(source);
		const ahash64 = deterministicAhash64Hex(bytes);
		const object = `ahash64-v1-${ahash64}`;
		await writeFile(path.join(objectsDirectory, object), bytes);
		files[`tex:${name}`] = {
			virtualPath: `/texlive/tex/browser/${name}`,
			object,
			ahash64,
			bytes: bytes.byteLength,
			dependencies,
		};
	}
	const manifest = {
		schema: 2,
		distribution: "umber-browser-fixture-v1",
		objectsBaseUrl,
		files,
	};
	await writeFile(
		path.join(directory, "manifest.json"),
		`${JSON.stringify(manifest)}\n`,
	);
	execFileSync(
		publisher,
		["--shard-existing", directory, "--shard-bits", "2"],
		{
			stdio: "pipe",
		},
	);
	const rootBytes = await readFile(path.join(directory, "manifest.json"));
	return {
		root: JSON.parse(rootBytes),
		rootAHash64: deterministicAhash64Hex(rootBytes),
	};
}
