import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const directory = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(directory, "../../..");
const packageDirectory = path.resolve(
	process.argv[2] ?? path.join(repository, "target/umber-wasm-package"),
);
const metadata = JSON.parse(
	await readFile(
		path.join(packageDirectory, "assets/plain-format.json"),
		"utf8",
	),
);
assert.equal(metadata.schema, 0);
assert.match(metadata.unavailable, /umber2-66p0\.27/);
console.log(
	"browser distribution integration: UNAVAILABLE pending deterministic aHash64 republication (umber2-66p0.27)",
);
