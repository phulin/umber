#![no_main]

use libfuzzer_sys::fuzz_target;
use tex_fonts::TfmFont;

fuzz_target!(|data: &[u8]| {
    let _ = TfmFont::parse(data);
});
