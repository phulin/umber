import assert from "node:assert/strict";
import test from "node:test";

import {
	renderedSourceKeyFromPoint,
	renderedSourceLocationFromPoint,
} from "./source-map.js";

function fixture({
	codes = "0x41,0x42",
	offset = 0,
	font = "7",
	textKind,
} = {}) {
	const page = element({
		umberPage: "2",
		umberRevision: "9",
		umberOutput: "0123456789abcdef0123456789abcdef",
	});
	const run = element({
		umberEvent: "4",
		umberCodes: codes,
		umberFont: font,
		umberTextKind: textKind,
	});
	const text = element();
	run.matches.set(".umber-page", page);
	text.matches.set(".umber-run", run);
	text.matches.set(".umber-run-text", text);
	const node = { parentElement: text };
	return {
		document: {
			caretPositionFromPoint(x, y) {
				assert.deepEqual([x, y], [12, 34]);
				return { offsetNode: node, offset };
			},
		},
	};
}

function element(dataset = {}) {
	return {
		dataset,
		matches: new Map(),
		closest(selector) {
			return this.matches.get(selector) ?? null;
		},
	};
}

function asciiEncoding() {
	const encoding = Array(256).fill(null);
	for (let code = 32; code <= 126; code += 1)
		encoding[code] = String.fromCodePoint(code);
	return encoding;
}

test("maps application-supplied encoding offsets to revision-bound rendered units", () => {
	const { document } = fixture({ offset: 1 });
	assert.deepEqual(
		renderedSourceKeyFromPoint(document, 12, 34, { encoding: asciiEncoding() }),
		{
			page: 2,
			event: 4,
			unit: 1,
			output: "0123456789abcdef0123456789abcdef",
			revision: 9,
		},
	);
});

test("uses UTF-16 lengths from the selected font encoding", () => {
	const encoding = Array(256).fill(null);
	encoding[1] = "𝒜";
	encoding[2] = "fi";
	encoding[3] = "Z";
	const options = { encodings: new Map([["7", encoding]]) };
	for (const [offset, unit] of [
		[0, 0],
		[1, 0],
		[2, 1],
		[3, 1],
		[4, 2],
		[5, 2],
	]) {
		const { document } = fixture({ codes: "0x01,0x02,0x03", offset });
		assert.equal(
			renderedSourceKeyFromPoint(document, 12, 34, options)?.unit,
			unit,
		);
	}
});

test("maps direct Unicode scalars, including surrogate-pair DOM offsets", () => {
	for (const [offset, unit] of [
		[0, 0],
		[1, 1],
		[2, 1],
		[3, 2],
	]) {
		const { document } = fixture({
			codes: "0x3b1,0x1d49c,0x416",
			offset,
			textKind: "unicode",
		});
		assert.equal(renderedSourceKeyFromPoint(document, 12, 34)?.unit, unit);
	}
});

test("preserves spaces as units and delegates one typed session query", () => {
	const { document } = fixture({ codes: "0x41,space,0x42", offset: 1 });
	const expected = {
		kind: "deleted",
		mintedRevision: 3,
	};
	const session = {
		renderedSourceLocation(...arguments_) {
			assert.deepEqual(arguments_, [
				2,
				4,
				1,
				"0123456789abcdef0123456789abcdef",
				9,
			]);
			return expected;
		},
	};
	assert.equal(
		renderedSourceLocationFromPoint(session, document, 12, 34, {
			encoding: asciiEncoding(),
		}),
		expected,
	);
});

test("returns null outside canonical text and for invalid metadata", () => {
	assert.equal(
		renderedSourceKeyFromPoint({ caretPositionFromPoint: () => null }, 12, 34),
		null,
	);
	const { document } = fixture({ codes: "bad", offset: 0 });
	assert.equal(renderedSourceKeyFromPoint(document, 12, 34), null);
	assert.throws(
		() => renderedSourceKeyFromPoint({}, 12, 34),
		/document\.caretPositionFromPoint is unavailable/,
	);
});
