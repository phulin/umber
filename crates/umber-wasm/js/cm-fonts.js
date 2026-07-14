const SHA256 =
	"1b875e541dc5c517cd11d244710d8639addbe91a0bb1ba55e7c4593225c7a970";

/** Returns an explicit OT1-like Unicode map for ordinary Computer Modern text. */
export function ot1TextEncoding() {
	const encoding = Array(256).fill(null);
	for (let code = 32; code <= 126; code += 1) {
		encoding[code] = String.fromCodePoint(code);
	}
	Object.assign(encoding, {
		0: "Γ",
		1: "Δ",
		2: "Θ",
		3: "Λ",
		4: "Ξ",
		5: "Π",
		6: "Σ",
		7: "Υ",
		8: "Φ",
		9: "Ψ",
		10: "Ω",
		16: "ı",
		17: "ȷ",
		25: "ß",
		26: "æ",
		27: "œ",
		28: "ø",
		29: "Æ",
		30: "Œ",
		31: "Ø",
	});
	return encoding;
}

/** Loads the packaged CM Unicode face for a caller-provided exact TFM identity. */
export async function loadComputerModernTextFont(
	name,
	tfmContentHash,
	options = {},
) {
	const fetchImpl = options.fetchImpl ?? globalThis.fetch;
	if (typeof fetchImpl !== "function")
		throw new TypeError("fetch is unavailable");
	const url = new URL("./assets/cmu-serif-500-roman.woff2", import.meta.url);
	const response = await fetchImpl(url);
	if (!response.ok)
		throw new Error(
			`Computer Modern font fetch failed: HTTP ${response.status}`,
		);
	return {
		name,
		tfmContentHash,
		woff2: new Uint8Array(await response.arrayBuffer()),
		sha256: SHA256,
		encoding: options.encoding ?? ot1TextEncoding(),
		provenance:
			"Computer Modern Unicode 0.7.0, SIL Open Font License 1.1; packaged from computer-modern@0.1.3",
		embeddable: true,
	};
}
