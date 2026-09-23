import assert from "node:assert/strict";
import test from "node:test";
import {
	classifyReadiness,
	createRustPrefetchPolicy,
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
		dependencyClosureRequest() {
			return [];
		}
		select() {
			return { hintKeys: [] };
		}
		admitRequest(request, _path, bytes) {
			calls.push(["admit", request, bytes]);
		}
		noteReplayRequest(region, request, discardedWork) {
			calls.push(["replay", region, request, discardedWork]);
			return { tier: 1, discardedWorkDelta: 7 };
		}
	}
	const policy = createRustPrefetchPolicy({
		prefetchLiteralHints: () => [],
		prefetchSelect: () => ({ hintKeys: [] }),
		prefetchPolicyVersion: () => "literal-groups-v1",
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
