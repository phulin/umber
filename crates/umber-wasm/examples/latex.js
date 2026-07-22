import {
	TEXLIVE_2026_MANIFEST_SHA256,
	TEXLIVE_2026_MANIFEST_URL,
} from "../manifest-resolver.js";
import { compileInWorker } from "../worker-controller.js";

const source = new TextEncoder().encode(
	"\\documentclass{article}\\begin{document}Hello from Umber LaTeX.\\end{document}",
);
const output = await compileInWorker(
	{ mainPath: "document.tex", jobName: "document", outputs: ["dvi"] },
	new Map([
		["document.tex", source],
		["document.aux", new Uint8Array()],
	]),
	{
		manifestUrl: TEXLIVE_2026_MANIFEST_URL,
		manifestSha256: TEXLIVE_2026_MANIFEST_SHA256,
		format: "latex",
		persistentCache: "indexeddb",
	},
);

document.querySelector("#status").textContent =
	`LaTeX-DVI: ${output.dvi.byteLength} bytes\n${output.terminal}`;
