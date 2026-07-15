import { compileInWorker } from "../worker-controller.js";

const source = new TextEncoder().encode(
	"\\documentclass{article}\\begin{document}Hello from Umber LaTeX.\\end{document}",
);
const output = await compileInWorker(
	{ mainPath: "document.tex", jobName: "document" },
	new Map([
		["document.tex", source],
		["document.aux", new Uint8Array()],
	]),
	{
		manifestUrl: new URL("./manifest.json", location.href).href,
		format: "latex",
		persistentCache: "indexeddb",
	},
);

document.querySelector("#status").textContent =
	`LaTeX-DVI: ${output.dvi.byteLength} bytes\n${output.terminal}`;
