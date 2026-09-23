import { readFile } from "node:fs/promises";

const metadata = JSON.parse(await readFile(process.argv[2], "utf8"));
if (metadata.schema === 0 && typeof metadata.unavailable === "string") {
	console.error(`default Plain format: BLOCKED (${metadata.unavailable})`);
	process.exitCode = 4;
} else if (metadata.schema === 12) {
	console.log("default Plain format: schema 12 metadata available");
} else {
	throw new Error(
		"default Plain format metadata has an unsupported availability state",
	);
}
