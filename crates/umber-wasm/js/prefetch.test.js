import assert from "node:assert/strict";
import test from "node:test";
import {
	classifyReadiness,
	extractLiteralHints,
	literalHintRequest,
	ResourceReadiness,
} from "./prefetch.js";

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
