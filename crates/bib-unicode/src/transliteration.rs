#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transliteration {
    Latin,
    DevanagariLatin,
}

pub fn transliterate(value: &str, scheme: Transliteration) -> String {
    match scheme {
        Transliteration::Latin => value.to_owned(),
        Transliteration::DevanagariLatin => devanagari_latin(value),
    }
}

fn devanagari_latin(value: &str) -> String {
    let mut output = String::new();
    for c in value.chars() {
        let mapped = match c {
            'क' => "k",
            'ख' => "kh",
            'ज' => "j",
            'त' => "t",
            'द' => "d",
            'व' => "v",
            'ा' => "ā",
            'ि' => "i",
            'ी' => "ī",
            'ृ' => "ṛ",
            '्' => "",
            'ष' => "ṣ",
            'ञ' => "ñ",
            _ => {
                output.push(c);
                continue;
            }
        };
        output.push_str(mapped);
    }
    output
}
