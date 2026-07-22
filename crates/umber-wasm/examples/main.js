import {
	TEXLIVE_2026_MANIFEST_SHA256,
	TEXLIVE_2026_MANIFEST_URL,
} from "../manifest-resolver.js";
import { compileInWorker } from "../worker-controller.js";

const source = new TextEncoder().encode("Hello from Umber.\\par\\bye");
const output = await compileInWorker(
	{ mainPath: "main.tex", outputs: ["dvi"] },
	new Map([["main.tex", source]]),
	{
		manifestUrl: TEXLIVE_2026_MANIFEST_URL,
		manifestSha256: TEXLIVE_2026_MANIFEST_SHA256,
		format: "plain",
		persistentCache: "indexeddb",
	},
);

document.querySelector("#status").textContent =
	`DVI: ${output.dvi.byteLength} bytes\n${output.terminal}`;
