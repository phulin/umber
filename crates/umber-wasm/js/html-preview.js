const REQUIRED_CSP = "default-src 'none'; font-src data: 'self'; style-src 'unsafe-inline'; img-src data:";

/** Installs Umber-generated standalone HTML in a scriptless iframe boundary. */
export function installHtmlPreview(iframe, html) {
	if (!iframe || typeof iframe.setAttribute !== "function") {
		throw new TypeError("preview target must be an iframe-like element");
	}
	const text = decode(html);
	validate(text);
	iframe.setAttribute("sandbox", "");
	iframe.setAttribute("referrerpolicy", "no-referrer");
	iframe.setAttribute("title", "Umber HTML preview");
	iframe.srcdoc = text;
	return iframe;
}

function decode(html) {
	if (typeof html === "string") return html;
	if (!(html instanceof Uint8Array)) {
		throw new TypeError("HTML preview bytes must be a Uint8Array or string");
	}
	return new TextDecoder("utf-8", { fatal: true }).decode(html);
}

function validate(html) {
	if (!html.startsWith("<!doctype html>\n<html ")) {
		throw new Error("preview is not canonical Umber HTML");
	}
	if (!html.includes('name="generator" content="umber-html/1"')) {
		throw new Error("preview has an unsupported Umber HTML schema");
	}
	if (!html.includes(`http-equiv="Content-Security-Policy" content="${REQUIRED_CSP}"`)) {
		throw new Error("preview is missing the required content security policy");
	}
	if (/<\s*(?:script|base|iframe|object|embed|form)\b/i.test(html)) {
		throw new Error("preview contains a forbidden active element");
	}
}
