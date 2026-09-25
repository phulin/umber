import { readFile } from "node:fs/promises";
import { deterministicAhash64Hex } from "../js/manifest-resolver.js";

const metadata = JSON.parse(await readFile(process.argv[2], "utf8"));
if (metadata.schema === 0 && typeof metadata.unavailable === "string") {
	console.error(`default Plain format: BLOCKED (${metadata.unavailable})`);
	process.exitCode = 4;
} else if (
	metadata.schema === 3 &&
	metadata.name === "plain" &&
	Number.isSafeInteger(metadata.formatSchema) &&
	metadata.formatSchema > 0 &&
	metadata.object === `ahash64-v1-${metadata.ahash64}` &&
	/^[0-9a-f]{16}$/.test(metadata.ahash64) &&
	Number.isSafeInteger(metadata.bytes) &&
	metadata.bytes > 0
) {
	const image = await readFile(process.argv[3]);
	if (
		image.byteLength < 12 ||
		image.subarray(0, 8).toString("binary") !== "UMBRFMT\0" ||
		image.readUInt32LE(8) !== metadata.formatSchema ||
		image.byteLength !== metadata.bytes ||
		deterministicAhash64Hex(image) !== metadata.ahash64
	) {
		throw new Error("default Plain format image does not match its metadata");
	}
	console.log(
		`default Plain format: schema ${metadata.formatSchema} image and metadata available`,
	);
} else {
	throw new Error(
		"default Plain format metadata has an unsupported availability state",
	);
}
