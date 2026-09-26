//! TeX82 §914's character immediately before a hyphenated word.

use smallvec::SmallVec;
use tex_state::ids::FontId;
use tex_state::node_view::NodeView;

use super::WordChar;

#[cfg(test)]
mod tests;

pub(super) struct ReconstitutionPrefix {
    pub(super) chars: SmallVec<[WordChar; 4]>,
    pub(super) no_left_boundary: bool,
}

/// TeX removes `ha` along with the word when the preceding node is a glyph
/// in the word's font. Its original characters seed `hu[0]` and `init_list`,
/// so reconstitution can restore a ligature or kern across that boundary.
pub(super) fn preceding_glyph(
    node: NodeView<'_>,
    word_font: FontId,
) -> Option<ReconstitutionPrefix> {
    match node {
        NodeView::Char { font, ch, origin } if font == word_font => {
            let mut chars = SmallVec::new();
            chars.push(WordChar {
                font,
                ch,
                lower: ch,
                origin,
            });
            Some(ReconstitutionPrefix {
                chars,
                no_left_boundary: true,
            })
        }
        NodeView::Lig {
            font,
            orig,
            origins,
            left_hit,
            ..
        } if font == word_font && orig.len() == origins.len() && (!orig.is_empty() || left_hit) => {
            // A ligature's source may span several characters. Replaying only
            // its displayed glyph would discard the source needed for TeX's
            // `init_list` and could change the next ligature or font kern.
            let chars = orig
                .iter()
                .enumerate()
                .map(|(index, &ch)| WordChar {
                    font,
                    ch,
                    lower: ch,
                    origin: origins[index],
                })
                .collect();
            Some(ReconstitutionPrefix {
                chars,
                no_left_boundary: !left_hit,
            })
        }
        _ => None,
    }
}
