//! Code pages of the viewer, and the system's, picked as Windows picks them.

use encoding_rs::Encoding;

/// The Encoding menu, as Total Commander's Lister has it.
pub const MENU: &[(&str, u16)] = &[
    ("MacOS", 10000),
    ("Eastern Europe (1250)", 1250),
    ("Cyrillic Windows (1251)", 1251),
    ("Western Latin1 (1252)", 1252),
    ("Greek (1253)", 1253),
    ("Turkish (1254)", 1254),
    ("Hebrew (1255)", 1255),
    ("Arabic (1256)", 1256),
    ("Baltic (1257)", 1257),
    ("Vietnamese (1258)", 1258),
    ("Chinese Main Land (936)", 936),
    ("Chinese Taiwan (950)", 950),
    ("Japanese (932)", 932),
    ("Korean (949)", 949),
    ("Thai (874)", 874),
    ("DOS-USA (437)", 437),
    ("DOS-LATIN1 (850)", 850),
    ("DOS Central Europe (852)", 852),
    ("DOS Cyrillic (866)", 866),
    ("Cyrillic KOI8-R (20866)", 20866),
];

pub fn decode(page: u16, bytes: &[u8]) -> String {
    let table = match page {
        437 => &CP437,
        850 => &CP850,
        852 => &CP852,
        _ => {
            let encoding = encoding(page).unwrap_or(encoding_rs::WINDOWS_1252);
            return encoding.decode_without_bom_handling(bytes).0.into_owned();
        }
    };
    bytes
        .iter()
        .map(|&byte| match byte {
            0x80.. => table[byte as usize - 0x80],
            _ => byte as char,
        })
        .collect()
}

/// The characters of `bytes`, each with how many bytes it takes.
pub fn chars(page: u16, bytes: &[u8]) -> Vec<(char, usize)> {
    let Some(encoding) = encoding(page).filter(|_| matches!(page, 932 | 936 | 949 | 950)) else {
        return decode(page, bytes).chars().map(|c| (c, 1)).collect();
    };
    // Fed a byte at a time, the decoder tells where each character ends.
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let (mut chars, mut start, mut out) = (Vec::new(), 0, String::with_capacity(16));
    for (i, byte) in bytes.iter().enumerate() {
        out.clear();
        let last = i + 1 == bytes.len();
        let _ = decoder.decode_to_string(std::slice::from_ref(byte), &mut out, last);
        for c in out.chars() {
            chars.push((c, i + 1 - start));
            start = i + 1;
        }
    }
    chars
}

fn encoding(page: u16) -> Option<&'static Encoding> {
    use encoding_rs::*;
    Some(match page {
        866 => IBM866,
        874 => WINDOWS_874,
        932 => SHIFT_JIS,
        936 => GBK,
        949 => EUC_KR,
        950 => BIG5,
        1250 => WINDOWS_1250,
        1251 => WINDOWS_1251,
        1252 => WINDOWS_1252,
        1253 => WINDOWS_1253,
        1254 => WINDOWS_1254,
        1255 => WINDOWS_1255,
        1256 => WINDOWS_1256,
        1257 => WINDOWS_1257,
        1258 => WINDOWS_1258,
        10000 => MACINTOSH,
        20866 => KOI8_R,
        _ => return None,
    })
}

/// The ANSI and the DOS code page: Windows' own; elsewhere by the language.
pub fn system() -> (u16, u16) {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetACP() -> u32;
            fn GetOEMCP() -> u32;
        }
        // SAFETY: both only read the system's settings.
        unsafe { (GetACP() as u16, GetOEMCP() as u16) }
    }
    #[cfg(not(windows))]
    {
        let lang = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
            .unwrap_or_default();
        by_language(&lang)
    }
}

/// As Windows sets them for a language: `hu_HU.UTF-8` is 1250 and 852.
#[cfg(any(not(windows), test))]
fn by_language(lang: &str) -> (u16, u16) {
    let lang = lang.to_ascii_lowercase();
    if lang.starts_with("zh_tw") || lang.starts_with("zh_hk") {
        return (950, 950);
    }
    match lang.get(..2).unwrap_or_default() {
        "hu" | "cs" | "pl" | "sk" | "sl" | "hr" | "ro" | "sq" | "bs" => (1250, 852),
        "ru" | "uk" | "bg" | "be" | "sr" | "mk" => (1251, 866),
        "el" => (1253, 437),
        "tr" | "az" => (1254, 437),
        "he" => (1255, 437),
        "ar" | "fa" | "ur" => (1256, 437),
        "lt" | "lv" | "et" => (1257, 437),
        "vi" => (1258, 1258),
        "th" => (874, 874),
        "ja" => (932, 932),
        "ko" => (949, 949),
        "zh" => (936, 936),
        "de" | "fr" | "es" | "it" | "pt" | "nl" | "da" | "sv" | "nb" | "nn" | "no" | "fi"
        | "is" | "ca" | "eu" | "gl" => (1252, 850),
        _ => (1252, 437),
    }
}

const CP437: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ',
    'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ',
    'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕',
    '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦',
    '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐',
    '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±',
    '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', ' ',
];
const CP850: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ',
    'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', 'ø', '£', 'Ø', '×', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ',
    'ª', 'º', '¿', '®', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', 'Á', 'Â', 'À', '©',
    '╣', '║', '╗', '╝', '¢', '¥', '┐', '└', '┴', '┬', '├', '─', '┼', 'ã', 'Ã', '╚', '╔', '╩', '╦',
    '╠', '═', '╬', '¤', 'ð', 'Ð', 'Ê', 'Ë', 'È', 'ı', 'Í', 'Î', 'Ï', '┘', '┌', '█', '▄', '¦', 'Ì',
    '▀', 'Ó', 'ß', 'Ô', 'Ò', 'õ', 'Õ', 'µ', 'þ', 'Þ', 'Ú', 'Û', 'Ù', 'ý', 'Ý', '¯', '´', '\u{ad}',
    '±', '‗', '¾', '¶', '§', '÷', '¸', '°', '¨', '·', '¹', '³', '²', '■', ' ',
];
const CP852: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'ů', 'ć', 'ç', 'ł', 'ë', 'Ő', 'ő', 'î', 'Ź', 'Ä', 'Ć', 'É', 'Ĺ', 'ĺ',
    'ô', 'ö', 'Ľ', 'ľ', 'Ś', 'ś', 'Ö', 'Ü', 'Ť', 'ť', 'Ł', '×', 'č', 'á', 'í', 'ó', 'ú', 'Ą', 'ą',
    'Ž', 'ž', 'Ę', 'ę', '¬', 'ź', 'Č', 'ş', '«', '»', '░', '▒', '▓', '│', '┤', 'Á', 'Â', 'Ě', 'Ş',
    '╣', '║', '╗', '╝', 'Ż', 'ż', '┐', '└', '┴', '┬', '├', '─', '┼', 'Ă', 'ă', '╚', '╔', '╩', '╦',
    '╠', '═', '╬', '¤', 'đ', 'Đ', 'Ď', 'Ë', 'ď', 'Ň', 'Í', 'Î', 'ě', '┘', '┌', '█', '▄', 'Ţ', 'Ů',
    '▀', 'Ó', 'ß', 'Ô', 'Ń', 'ń', 'ň', 'Š', 'š', 'Ŕ', 'Ú', 'ŕ', 'Ű', 'ý', 'Ý', 'ţ', '´', '\u{ad}',
    '˝', '˛', 'ˇ', '˘', '§', '÷', '¸', '°', '¨', '˙', 'ű', 'Ř', 'ř', '■', ' ',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_code_pages() {
        let hungarian = b"\xc1rv\xedzt\xfbr\xf5";
        assert_eq!(decode(1250, hungarian), "Árvíztűrő");
        assert_eq!(decode(1252, hungarian), "Árvíztûrõ");
        assert_eq!(decode(852, b"\x8a\xfb"), "Őű");
        assert_eq!(decode(437, b"\xc9\xcd\xbb"), "╔═╗");
        assert_eq!(by_language("hu_HU.UTF-8"), (1250, 852));
        assert_eq!(by_language("C"), (1252, 437));
        for &(_, page) in MENU {
            assert!(!decode(page, b"abc").is_empty(), "{page}");
        }
        assert_eq!(chars(1250, b"\xc1r"), [('Á', 1), ('r', 1)]);
        let japanese = encoding_rs::SHIFT_JIS.encode("a日本").0;
        assert_eq!(chars(932, &japanese), [('a', 1), ('日', 2), ('本', 2)]);
    }
}
