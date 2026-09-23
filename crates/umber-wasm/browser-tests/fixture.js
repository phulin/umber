import { HttpManifestResolver } from "/package/manifest-resolver.js";
import * as bindings from "/package/umber_wasm.js";
import { compileInWorker } from "/package/worker-controller.js";

globalThis.umberBrowserIntegration = (async () => {
	const digest = new URL(location.href).searchParams.get("digest");
	await bindings.default({ module_or_path: "/package/umber_wasm_bg.wasm" });
	const resolver = await HttpManifestResolver.create({
		manifestUrl: "/publication/manifest.json",
		manifestAHash64: digest,
		catalog: bindings,
	});
	const responses = await resolver.resolve([
		{ kind: "tex", name: "probe.tex" },
	]);
	if (responses.length !== 1 || responses[0].type !== "file") {
		throw new Error("browser packed catalog did not resolve probe.tex");
	}
	const output = await compileInWorker(
		{ mainPath: "/job/main.tex", outputs: ["dvi"] },
		new Map([
			[
				"/job/main.tex",
				new TextEncoder().encode("\\input probe.tex\\shipout\\hbox{N}\\end"),
			],
		]),
		{ manifestUrl: "/publication/manifest.json", manifestAHash64: digest },
		{ wasmUrl: "/package/umber_wasm_bg.wasm", timeoutMs: 60_000 },
	);
	if (
		output.dvi.byteLength === 0 ||
		!new TextDecoder().decode(output.log).includes("PACKED-CATALOG-RESOURCE")
	) {
		throw new Error("packaged worker failed the real WASM resource round trip");
	}
	return { catalog: "packed-rust", worker: "resource-round-trip" };
})();
