import { compileInWorker } from "../worker-controller.js";

const source = new TextEncoder().encode("Hello from Umber.\\par\\bye");
const output = await compileInWorker(
	{ mainPath: "main.tex" },
	new Map([["main.tex", source]]),
	{
		manifestUrl: new URL("./manifest.json", location.href).href,
		format: "plain",
		persistentCache: "indexeddb",
	},
);

document.querySelector("#status").textContent =
	`DVI: ${output.dvi.byteLength} bytes\n${output.terminal}`;
