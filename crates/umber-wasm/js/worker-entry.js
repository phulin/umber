import { compile } from "./compile.js";
import { HttpManifestResolver } from "./manifest-resolver.js";

export async function runCompileMessage(message, dependencies = {}) {
	if (message?.kind !== "compile" || !Array.isArray(message.userFiles)) {
		throw new Error("invalid compile worker request");
	}
	const bindings = dependencies.bindings ?? (await import("./umber_wasm.js"));
	if (dependencies.bindings === undefined) {
		const initialize = bindings.default;
		await initialize(
			message.wasmUrl === undefined
				? undefined
				: { module_or_path: message.wasmUrl },
		);
	}
	const resolver =
		dependencies.resolver ??
		(await HttpManifestResolver.create(message.resolver));
	return compile(
		message.options,
		new Map(message.userFiles),
		resolver,
		undefined,
		bindings,
	);
}

export function outputTransfers(output) {
	const transfers = [output.log.buffer, output.dvi.buffer];
	for (const file of output.files) transfers.push(file.bytes.buffer);
	return [...new Set(transfers)];
}

const scope = globalThis;
const isWorkerRealm =
	typeof WorkerGlobalScope !== "undefined" &&
	scope instanceof WorkerGlobalScope;
if (isWorkerRealm) {
	let started = false;
	scope.addEventListener("message", async (event) => {
		if (started) return;
		started = true;
		try {
			const output = await runCompileMessage(event.data);
			scope.postMessage({ kind: "complete", output }, outputTransfers(output));
		} catch (error) {
			scope.postMessage({ kind: "error", error: serializedError(error) });
		}
	});
}

function serializedError(error) {
	return {
		name: error instanceof Error ? error.name : "Error",
		code: error?.code,
		message: error instanceof Error ? error.message : String(error),
		diagnostic: error?.diagnostic,
		stack: error instanceof Error ? error.stack : undefined,
	};
}
