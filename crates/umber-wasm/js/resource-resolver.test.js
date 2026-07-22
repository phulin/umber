import assert from "node:assert/strict";
import test from "node:test";
import { CompositeResourceResolver } from "./resource-resolver.js";

const file = (name, kind = "tex") => ({
	type: "file",
	domain: "tex",
	kind,
	name,
	originalName: name,
});
const unavailable = (request) => ({
	...request,
	type: `${request.type}-unavailable`,
});
const resolved = (request, byte) => ({
	...request,
	type: "file",
	virtualPath: `/objects/${byte}`,
	bytes: new Uint8Array([byte]),
});

test("higher-precedence positives shadow later providers and misses fall through", async () => {
	const calls = [];
	const first = {
		async resolve(requests) {
			calls.push(["first", requests.map(({ name }) => name)]);
			return requests.map((request) =>
				request.name === "private.tex"
					? resolved(request, 1)
					: unavailable(request),
			);
		},
	};
	const second = {
		async resolve(requests) {
			calls.push(["second", requests.map(({ name }) => name)]);
			return requests.map((request) => resolved(request, 2));
		},
	};
	const resolver = new CompositeResourceResolver([first, second]);
	const responses = await resolver.resolve([
		file("private.tex"),
		file("hosted.tex"),
	]);
	assert.deepEqual(
		responses.map(({ bytes }) => [...bytes]),
		[[1], [2]],
	);
	assert.deepEqual(calls, [
		["first", ["private.tex", "hosted.tex"]],
		["second", ["hosted.tex"]],
	]);
});

test("absence becomes authoritative only after every provider misses", async () => {
	const request = file("cmr17.tfm", "tfm");
	let calls = 0;
	const miss = {
		async resolve(requests) {
			calls += 1;
			return requests.map(unavailable);
		},
	};
	const resolver = new CompositeResourceResolver([miss, miss]);
	assert.deepEqual(await resolver.resolve([request]), [unavailable(request)]);
	assert.equal(calls, 2);
});

test("transport failure is actionable and never converted to absence", async () => {
	const expected = new Error("offline object missing");
	const resolver = new CompositeResourceResolver([
		{
			async resolve() {
				throw expected;
			},
		},
		{
			async resolve(requests) {
				return requests.map(unavailable);
			},
		},
	]);
	await assert.rejects(resolver.resolve([file("plain.tex")]), expected);
});

test("complete typed keys prevent basename and semantic-kind aliasing", async () => {
	const tex = file("cmr10", "tex");
	const tfm = file("cmr10", "tfm");
	const provider = {
		async resolve(requests) {
			return requests.map((request) =>
				request.kind === "tfm" ? resolved(request, 3) : unavailable(request),
			);
		},
	};
	const responses = await new CompositeResourceResolver([provider]).resolve([
		tex,
		tfm,
	]);
	assert.equal(responses[0].type, "file-unavailable");
	assert.deepEqual([...responses[1].bytes], [3]);
});

test("cancellation prevents a later provider from accepting resources", async () => {
	const controller = new AbortController();
	let laterCalled = false;
	const resolver = new CompositeResourceResolver([
		{
			async resolve(requests) {
				controller.abort(new Error("superseded"));
				return requests.map(unavailable);
			},
		},
		{
			async resolve() {
				laterCalled = true;
				return [];
			},
		},
	]);
	await assert.rejects(
		resolver.resolve([file("plain.tex")], { signal: controller.signal }),
		/superseded/,
	);
	assert.equal(laterCalled, false);
});

test("unexpected and duplicate provider bindings are rejected", async () => {
	const request = file("plain.tex");
	await assert.rejects(
		new CompositeResourceResolver([
			{
				async resolve() {
					return [resolved(file("other.tex"), 1)];
				},
			},
		]).resolve([request]),
		/unexpected response/,
	);
	await assert.rejects(
		new CompositeResourceResolver([
			{
				async resolve() {
					return [resolved(request, 1), resolved(request, 1)];
				},
			},
		]).resolve([request]),
		/duplicate response/,
	);
});

test("PK requests preserve byte names, DPI, and frozen mode", async () => {
	const request = {
		type: "pk-font",
		texName: new Uint8Array([0x63, 0x6d, 0x72, 0x31, 0x30]),
		dpi: 600,
		mode: new Uint8Array([0x6c, 0x6a, 0x66, 0x6f, 0x75, 0x72]),
	};
	const responses = await new CompositeResourceResolver([
		{
			async resolve(requests) {
				return requests.map(unavailable);
			},
		},
	]).resolve([request]);
	assert.equal(responses[0].type, "pk-font-unavailable");
	assert.deepEqual(responses[0].texName, request.texName);
	assert.deepEqual(responses[0].mode, request.mode);
	assert.equal(responses[0].dpi, 600);
});
