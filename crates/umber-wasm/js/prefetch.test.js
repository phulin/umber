import assert from "node:assert/strict";
import test from "node:test";
import {
	classifyReadiness,
	createRustPrefetchPolicy,
	extractLiteralHints,
	literalHintRequest,
	ResourceReadiness,
} from "./prefetch.js";

test("Rust policy adapter keeps request identity at the WASM boundary", () => {
	const calls = [];
	class FakePrefetchPolicySession {
		enqueue(requests) {
			calls.push(["enqueue", requests]);
		}
		enqueueEscalation(requests, priority) {
			calls.push(["escalation", requests, priority]);
		}
		enqueueLiteralHints(source) {
			calls.push(["literal", source]);
			return 1;
		}
		drain() {
			return [
				{
					key: "tex:child.sty",
					domain: "tex",
					kind: "tex",
					name: "child.sty",
					originalSpelling: "./child.sty",
					searchContext: "runtime",
					class: "small-runtime",
					required: false,
					depth: 1,
				},
			];
		}
		dependencyClosure() {
			return [];
		}
		admit(key, bytes) {
			calls.push(["admit", key, bytes]);
		}
		noteReplay(region, requestKey, discardedWork) {
			calls.push(["replay", region, requestKey, discardedWork]);
			return { tier: 1, discardedWorkDelta: 7 };
		}
	}
	const policy = createRustPrefetchPolicy({
		prefetchLiteralHints: () => [],
		prefetchSelect: () => ({ hintKeys: [] }),
		PrefetchPolicySession: FakePrefetchPolicySession,
	});
	assert.equal(policy.version, "literal-groups-v1");
	const state = policy.createState();
	state.enqueue([
		{
			type: "file",
			kind: "tex",
			name: "foo.sty",
			originalName: "foo.sty",
			searchContext: "accepted",
		},
	]);
	state.enqueueLiteralHints("\\input{child.sty}");
	assert.deepEqual(state.drain(1), [
		{
			type: "file",
			domain: "tex",
			kind: "tex",
			name: "child.sty",
			originalName: "./child.sty",
			searchContext: "runtime",
			depth: 1,
		},
	]);
	const request = {
		type: "file",
		domain: "tex",
		kind: "tex",
		name: "foo.sty",
	};
	const bytes = new Uint8Array([1, 2]);
	state.admit(request, "/tex/foo.sty", bytes, []);
	assert.deepEqual(state.noteReplay("region", request, 9), {
		tier: 1,
		discardedWorkDelta: 7,
	});
	assert.equal(calls[0][0], "enqueue");
	assert.equal(calls[0][1][0].key, "tex:foo.sty");
	assert.deepEqual(
		{
			domain: calls[0][1][0].domain,
			kind: calls[0][1][0].kind,
			name: calls[0][1][0].name,
		},
		{ domain: "tex", kind: "tex", name: "foo.sty" },
	);
	assert.equal(calls.at(-1)[0], "replay");
});

test("literal source hints ignore comments, malformed arguments, and dynamic forms", () => {
	const hints = extractLiteralHints(`
\\documentclass[11pt]{article}
% \\usepackage{ignored}
\\usepackage{amsmath, graphicx}
\\input{chapters/one}
\\includegraphics[width=2cm]{figures/plot}
\\input\\dynamic
\\input{unterminated
`);
	assert.deepEqual(
		hints.map(({ kind, name }) => ({ kind, name })),
		[
			{ kind: "documentclass", name: "article" },
			{ kind: "package", name: "amsmath" },
			{ kind: "package", name: "graphicx" },
			{ kind: "input", name: "chapters/one" },
			{ kind: "includegraphics", name: "figures/plot" },
		],
	);
	assert.deepEqual(literalHintRequest(hints.at(-1)), {
		type: "file",
		domain: "tex",
		kind: "image",
		name: "figures/plot",
		originalName: "figures/plot",
	});
});

test("readiness never turns an unknown transport result into absence", () => {
	assert.equal(
		classifyReadiness({ exists: true }),
		ResourceReadiness.ExistsNotReady,
	);
	assert.equal(
		classifyReadiness({ exists: true, payloadAdmitted: true }),
		ResourceReadiness.Ready,
	);
	assert.equal(
		classifyReadiness({ exists: false, authoritativeAbsent: true }),
		ResourceReadiness.Absent,
	);
	assert.equal(classifyReadiness({ exists: false }), undefined);
	assert.equal(classifyReadiness({}), undefined);
});
