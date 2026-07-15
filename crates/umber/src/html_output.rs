use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tex_out::FontResource;
use tex_out::html::{HtmlFontKey, HtmlFontResolver, WebFont};

/// Native driver resolver for an explicit, deterministic web-font bundle.
///
/// For font `cmr10`, the directory contains `cmr10.woff2`,
/// `cmr10.woff2.sha256`, `cmr10.tfm-hash`, `cmr10.map`, and `cmr10.license`.
/// The map has `HH<TAB>UTF-8` lines; `-` denotes an intentionally unmapped
/// code. All 256 codes must occur exactly once.
pub struct DirectoryHtmlFontResolver<'a> {
    root: PathBuf,
    world: &'a mut tex_state::World,
}

impl<'a> DirectoryHtmlFontResolver<'a> {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, world: &'a mut tex_state::World) -> Self {
        Self {
            root: root.into(),
            world,
        }
    }
}

impl HtmlFontResolver for DirectoryHtmlFontResolver<'_> {
    fn resolve(&mut self, font: &FontResource) -> Result<WebFont, String> {
        let stem = safe_stem(&font.name)?;
        let tfm_hash = read_text(self.world, &self.root.join(format!("{stem}.tfm-hash")))?;
        if tfm_hash.trim() != font.tfm_content_hash.hex() {
            return Err(format!("TFM content hash mismatch for {}", font.name));
        }
        let woff2 = read(self.world, &self.root.join(format!("{stem}.woff2")))?;
        let expected = parse_digest(&read_text(
            self.world,
            &self.root.join(format!("{stem}.woff2.sha256")),
        )?)?;
        let actual: [u8; 32] = Sha256::digest(&woff2).into();
        if actual != expected {
            return Err(format!("WOFF2 SHA-256 mismatch for {}", font.name));
        }
        let encoding = parse_map(&read_text(
            self.world,
            &self.root.join(format!("{stem}.map")),
        )?)?;
        let provenance = read_text(self.world, &self.root.join(format!("{stem}.license")))?;
        if provenance.trim().is_empty() {
            return Err(format!("empty embedding license for {}", font.name));
        }
        Ok(WebFont {
            key: HtmlFontKey::from(font),
            woff2,
            sha256: actual,
            encoding,
            provenance,
            embeddable: true,
        })
    }
}

fn safe_stem(name: &str) -> Result<&str, String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Err(format!("unsafe web-font bundle name {name:?}"))
    } else {
        Ok(name)
    }
}

fn read(world: &mut tex_state::World, path: &Path) -> Result<Vec<u8>, String> {
    world
        .read_file(path)
        .map(|content| content.bytes().to_vec())
        .map_err(|error| error.to_string())
}

fn read_text(world: &mut tex_state::World, path: &Path) -> Result<String, String> {
    String::from_utf8(read(world, path)?).map_err(|_| format!("{} is not UTF-8", path.display()))
}

fn parse_digest(value: &str) -> Result<[u8; 32], String> {
    let value = value.trim();
    if value.len() != 64 {
        return Err("WOFF2 SHA-256 must contain 64 lowercase hex digits".to_owned());
    }
    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(digest)
}

fn nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err("WOFF2 SHA-256 must use lowercase hex".to_owned()),
    }
}

fn parse_map(value: &str) -> Result<Vec<Option<String>>, String> {
    let mut mapping = vec![None; 256];
    let mut seen = [false; 256];
    for line in value.lines() {
        let (code, text) = line
            .split_once('\t')
            .ok_or_else(|| "encoding lines must be HH<TAB>text".to_owned())?;
        if code.len() != 2 {
            return Err(format!("invalid encoding code {code:?}"));
        }
        let index = usize::from((nibble(code.as_bytes()[0])? << 4) | nibble(code.as_bytes()[1])?);
        if std::mem::replace(&mut seen[index], true) {
            return Err(format!("duplicate encoding code {code}"));
        }
        if text != "-" {
            mapping[index] = Some(text.to_owned());
        }
    }
    if let Some(index) = seen.iter().position(|seen| !seen) {
        return Err(format!("encoding omits code {index:02x}"));
    }
    Ok(mapping)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_arith::Scaled;
    use tex_out::ContentHash;

    #[test]
    fn directory_bundle_checks_identity_and_complete_mapping() {
        let root = Path::new("/fonts");
        let mut world = tex_state::World::memory();
        let bytes = b"woff2 fixture";
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        world
            .set_memory_file(root.join("cmr10.woff2"), bytes.to_vec())
            .expect("woff");
        world
            .set_memory_file(root.join("cmr10.woff2.sha256"), hex(&digest).into_bytes())
            .expect("digest");
        let tfm = ContentHash::from_bytes(b"tfm");
        world
            .set_memory_file(root.join("cmr10.tfm-hash"), tfm.hex().into_bytes())
            .expect("tfm hash");
        world
            .set_memory_file(root.join("cmr10.license"), b"test license".to_vec())
            .expect("license");
        let map = (0..=255)
            .map(|code| format!("{code:02x}\t{}", if code == 65 { "A" } else { "-" }))
            .collect::<Vec<_>>()
            .join("\n");
        world
            .set_memory_file(root.join("cmr10.map"), map.into_bytes())
            .expect("map");
        let font = FontResource {
            font_id: 1,
            name: "cmr10".to_owned(),
            tfm_content_hash: tfm,
            tfm_checksum: 0,
            design_size: Scaled::from_raw(1),
            at_size: Scaled::from_raw(1),
            opentype: None,
            semantic_identity: tex_fonts::FontSourceIdentity::from_bytes(tfm.bytes()),
            construction: tex_out::FontResourceConstruction::Loaded,
        };
        let web = DirectoryHtmlFontResolver::new(root, &mut world)
            .resolve(&font)
            .expect("resolve");
        assert_eq!(web.encoding[65].as_deref(), Some("A"));
        assert_eq!(web.sha256, digest);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
