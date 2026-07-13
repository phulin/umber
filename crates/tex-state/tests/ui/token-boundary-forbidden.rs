use tex_state::token::{OriginId, TracedTokenWord};

fn main() {
    let origin = OriginId::from_raw(123);
    let word = TracedTokenWord::from_raw(456);
    let _origin_raw = origin.raw();
    let _word_raw = word.raw();
}
