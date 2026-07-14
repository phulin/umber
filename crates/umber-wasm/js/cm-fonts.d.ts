import type { HtmlFontInput } from "./umber_wasm.js";

export function ot1TextEncoding(): Array<string | null>;
export function loadComputerModernTextFont(
	name: string,
	tfmContentHash: string,
	options?: {
		fetchImpl?: typeof fetch;
		encoding?: Array<string | null>;
	},
): Promise<HtmlFontInput>;
