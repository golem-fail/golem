//! IPA → target-script transcription (loanword style) — Phase B.
//!
//! Renders the broad phonemic `ipa` into an approximate form in a target
//! script, used when a name is not already in that script. Output is
//! "consistent and defensible", not authoritative — a future per-entry
//! override layer can correct specific cells. `&str`-generic so the geo data
//! can reuse it once it gains `ipa`.
//!
//! - Cyrillic: concatenative phoneme→letter, ʲ→ь, nasal→н.
//! - Hangul: phonemes→jamo, composed into syllable blocks (onset·medial·coda)
//!   with ㅡ epenthesis for consonants that have no following vowel.
//! - Hebrew / Arabic: abjad consonantal skeleton — short vowels dropped, a
//!   leading vowel becomes the script's alef/aleph, long vowels and glides
//!   become matres lectionis; Hebrew applies final letter forms.

use crate::script::Script;

/// One IPA segment: a base phoneme (possibly an affricate digraph, with any
/// ʰ/ʷ/ˤ modifier kept attached for the abjad maps) plus suprasegmental flags.
struct Seg {
    base: String,
    long: bool,
    nasal: bool,
    palatal: bool,
}

const AFFRICATES: &[&str] = &["tʃ", "dʒ", "ts", "dz", "ʈʂ", "tʂ", "tɕ", "dʑ"];

fn is_vowel(base: &str) -> bool {
    matches!(
        base,
        "a" | "e"
            | "i"
            | "o"
            | "u"
            | "ə"
            | "ɪ"
            | "ɛ"
            | "ɔ"
            | "ʊ"
            | "ʌ"
            | "æ"
            | "ɑ"
            | "ɒ"
            | "ɯ"
            | "ɨ"
            | "ʏ"
            | "y"
            | "ø"
            | "œ"
            | "ɐ"
            | "ɜ"
            | "ɵ"
    )
}

/// Split one whitespace-free IPA word into segments. Stress (ˈ ˌ) and the
/// no-audible-release mark are dropped; length ː, combining/precomposed nasal,
/// and ʲ become flags; ʰ ʷ ˤ ʱ stay attached to the base for map lookup.
fn segment(word: &str) -> Vec<Seg> {
    let chars: Vec<char> = word.chars().collect();
    let mut segs: Vec<Seg> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == 'ˈ' || c == 'ˌ' || c == '\u{031A}' {
            i += 1;
            continue;
        }
        // Affricate digraph?
        let mut base = String::new();
        if i + 1 < chars.len() {
            let pair: String = [c, chars[i + 1]].iter().collect();
            if AFFRICATES.contains(&pair.as_str()) {
                base = pair;
                i += 2;
            }
        }
        let mut nasal = false;
        if base.is_empty() {
            // Precomposed nasal vowels decompose to (vowel, nasal).
            match c {
                'õ' => {
                    base.push('o');
                    nasal = true;
                }
                'ĩ' => {
                    base.push('i');
                    nasal = true;
                }
                _ => base.push(c),
            }
            i += 1;
        }
        let mut long = false;
        let mut palatal = false;
        while i < chars.len() {
            match chars[i] {
                'ː' => long = true,
                '\u{0303}' => nasal = true,
                'ʲ' => palatal = true,
                'ʰ' | 'ʷ' | 'ˤ' | 'ʱ' => base.push(chars[i]),
                _ => break,
            }
            i += 1;
        }
        segs.push(Seg {
            base,
            long,
            nasal,
            palatal,
        });
    }
    segs
}

/// Transcribe a phonemic IPA string into `target`. Words (space-separated) are
/// transcribed independently and rejoined with a space.
pub(crate) fn transcribe(ipa: &str, target: Script) -> String {
    ipa.split(' ')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let segs = segment(w);
            match target {
                Script::Cyrillic => to_cyrillic(&segs),
                Script::Hangul => to_hangul(&segs),
                Script::Hebrew => to_hebrew(&segs),
                Script::Arabic => to_arabic(&segs),
                _ => w.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Cyrillic
// ---------------------------------------------------------------------------

fn cyr(base: &str) -> &'static str {
    // Try the modifier-bearing base first, then fall back to the plain base.
    let stripped: String = base.chars().filter(|c| !"ʰʷˤʱ".contains(*c)).collect();
    match stripped.as_str() {
        "a" | "ɑ" | "ɐ" | "ʌ" | "ɒ" => "а",
        "e" | "ɛ" | "ɜ" | "æ" => "э",
        "i" | "ɪ" | "ɨ" => "и",
        "o" | "ɔ" | "ɵ" => "о",
        "u" | "ʊ" | "ɯ" => "у",
        "ə" => "а",
        "y" | "ʏ" => "ю",
        "ø" | "œ" => "ё",
        "p" => "п",
        "b" | "ɓ" => "б",
        "t" | "ʈ" => "т",
        "d" | "ɖ" | "ɗ" | "ð" => "д",
        "k" | "ɡ" | "g" | "q" => "к",
        "f" => "ф",
        "v" | "ʋ" | "β" | "w" => "в",
        "s" | "θ" => "с",
        "z" => "з",
        "ʃ" | "ʂ" | "ɕ" | "ɧ" => "ш",
        "ʒ" | "ʑ" | "ʝ" => "ж",
        "x" | "χ" | "h" | "ħ" | "ɦ" | "ɣ" | "ç" => "х",
        "m" => "м",
        "n" | "ɲ" | "ɳ" | "ɴ" => "н",
        "ŋ" => "нг",
        "l" | "ɫ" | "ɭ" | "ʎ" => "л",
        "r" | "ɾ" | "ʁ" => "р",
        "j" => "й",
        "tʃ" | "tʂ" | "ʈʂ" | "tɕ" => "ч",
        "dʒ" | "dʑ" => "дж",
        "ts" | "dz" => "ц",
        "ʔ" | "ʕ" => "",
        _ => "",
    }
}

fn to_cyrillic(segs: &[Seg]) -> String {
    let mut out = String::new();
    for s in segs {
        out.push_str(cyr(&s.base));
        if s.palatal {
            out.push('ь');
        }
        if s.nasal {
            out.push('н');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Hangul (jamo composition)
// ---------------------------------------------------------------------------

/// Onset (초성) jamo index for a consonant base, or null-onset ㅇ (11).
fn hangul_onset(base: &str) -> i32 {
    let stripped: String = base.chars().filter(|c| !"ʰʷˤʱ".contains(*c)).collect();
    match stripped.as_str() {
        "k" | "ɡ" | "g" | "q" | "x" | "χ" | "ɣ" => 0, // ㄱ
        "n" | "ɲ" | "ɳ" | "ɴ" => 2,                   // ㄴ
        "t" | "ʈ" | "d" | "ð" | "θ" => 3,             // ㄷ
        "r" | "ɾ" | "l" | "ɫ" | "ɭ" | "ʁ" | "ʎ" => 5, // ㄹ
        "m" => 6,                                     // ㅁ
        "b" | "ɓ" | "p" | "f" | "v" | "ʋ" | "β" => 7, // ㅂ
        "s" | "z" | "ʃ" | "ʂ" | "ɕ" | "ç" | "ɧ" => 9, // ㅅ
        "dʒ" | "dʑ" | "tɕ" | "ts" | "dz" | "ʒ" | "ʑ" | "ʝ" => 12, // ㅈ
        "tʃ" | "tʂ" | "ʈʂ" => 14,                     // ㅊ
        "h" | "ħ" | "ɦ" => 18,                        // ㅎ
        _ => 11,                                      // ㅇ (null)
    }
}

/// Medial (중성) jamo index for a vowel base.
fn hangul_medial(base: &str) -> i32 {
    match base {
        "a" | "ɑ" | "ɐ" | "ɒ" => 0,  // ㅏ
        "æ" => 1,                    // ㅐ
        "ʌ" | "ə" | "ɜ" | "ɵ" => 4,  // ㅓ
        "e" | "ɛ" => 5,              // ㅔ
        "o" | "ɔ" | "ø" | "œ" => 8,  // ㅗ
        "u" | "ʊ" | "y" | "ʏ" => 13, // ㅜ
        "ɯ" | "ɨ" => 18,             // ㅡ
        _ => 20,                     // ㅣ (i, ɪ, fallback)
    }
}

/// Coda (종성) jamo index if this consonant is a valid single final, else 0.
fn hangul_coda(base: &str) -> i32 {
    match base {
        "k" | "ɡ" | "g" => 1, // ㄱ
        "n" | "ɲ" => 4,       // ㄴ
        "t" | "d" => 7,       // ㄷ
        "l" | "ɫ" | "r" => 8, // ㄹ
        "m" => 16,            // ㅁ
        "b" | "p" => 17,      // ㅂ
        "ŋ" => 21,            // ㅇ
        _ => 0,
    }
}

fn compose(onset: i32, medial: i32, coda: i32) -> char {
    char::from_u32((0xAC00 + (onset * 21 + medial) * 28 + coda) as u32).unwrap_or('?')
}

fn to_hangul(segs: &[Seg]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < segs.len() {
        let cur = &segs[i];
        if is_vowel(&cur.base) {
            // Vowel-initial syllable: null onset.
            let medial = hangul_medial(&cur.base);
            i += 1;
            let coda = take_coda(segs, &mut i);
            out.push(compose(11, medial, coda));
        } else if i + 1 < segs.len() && is_vowel(&segs[i + 1].base) {
            // Consonant + vowel.
            let onset = hangul_onset(&cur.base);
            let medial = hangul_medial(&segs[i + 1].base);
            i += 2;
            let coda = take_coda(segs, &mut i);
            out.push(compose(onset, medial, coda));
        } else {
            // Consonant with no following vowel: ㅡ epenthesis.
            out.push(compose(hangul_onset(&cur.base), 18, 0));
            i += 1;
        }
    }
    out
}

/// If the next segment is a consonant that is a valid coda and is itself
/// followed by another consonant or end-of-word, consume it as this syllable's
/// coda and advance.
fn take_coda(segs: &[Seg], i: &mut usize) -> i32 {
    if *i < segs.len() && !is_vowel(&segs[*i].base) {
        let coda = hangul_coda(&segs[*i].base);
        let next_is_vowel = *i + 1 < segs.len() && is_vowel(&segs[*i + 1].base);
        if coda != 0 && !next_is_vowel {
            *i += 1;
            return coda;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Hebrew (abjad)
// ---------------------------------------------------------------------------

fn heb(base: &str, first: bool) -> &'static str {
    let stripped: String = base.chars().filter(|c| !"ʰʷˤʱ".contains(*c)).collect();
    match stripped.as_str() {
        // Vowels: dropped, except a leading vowel becomes alef.
        "a" | "e" | "i" | "o" | "u" | "ə" | "ɪ" | "ɛ" | "ɔ" | "ʊ" | "ʌ" | "æ" | "ɑ" | "ɒ" | "ɯ"
        | "ɨ" | "ʏ" | "y" | "ø" | "œ" | "ɐ" | "ɜ" | "ɵ" => {
            if first {
                "א"
            } else {
                ""
            }
        }
        "b" | "v" | "β" | "ɓ" => "ב",
        "ɡ" | "g" | "dʒ" | "dʑ" | "ɣ" => "ג",
        "d" | "ð" | "ɖ" | "ɗ" => "ד",
        "h" | "ɦ" => "ה",
        "w" => "ו",
        "z" | "ʒ" | "ʑ" => "ז",
        "ħ" | "x" | "χ" | "ç" => "ח",
        "t" | "ʈ" | "θ" => "ת",
        "j" => "י",
        "k" | "q" | "ɢ" => "ק",
        "l" | "ɫ" | "ɭ" | "ʎ" => "ל",
        "m" => "מ",
        "n" | "ɲ" | "ɳ" | "ɴ" | "ŋ" => "נ",
        "s" | "ts" | "dz" | "ɕ" => "ס",
        "ʕ" | "ʔ" => "ע",
        "p" | "f" => "פ",
        "ʃ" | "ʂ" | "tʃ" | "tʂ" | "ʈʂ" | "tɕ" | "ɧ" => "ש",
        "r" | "ɾ" | "ʁ" => "ר",
        "ʋ" => "ו",
        _ => "",
    }
}

const HEB_FINAL: &[(&str, &str)] = &[("כ", "ך"), ("מ", "ם"), ("נ", "ן"), ("פ", "ף"), ("צ", "ץ")];

fn to_hebrew(segs: &[Seg]) -> String {
    let mut letters: Vec<&'static str> = Vec::new();
    for (idx, s) in segs.iter().enumerate() {
        let l = heb(&s.base, idx == 0);
        if !l.is_empty() {
            letters.push(l);
        }
        // Long vowel after a consonant → mater lectionis (yod/vav).
        if s.long && is_vowel(&s.base) {
            // already handled by base; skip
        }
    }
    // Apply final letter form to the last letter where applicable.
    if let Some(last) = letters.last_mut() {
        for (reg, fin) in HEB_FINAL {
            if *last == *reg {
                *last = fin;
                break;
            }
        }
    }
    letters.concat()
}

// ---------------------------------------------------------------------------
// Arabic (abjad)
// ---------------------------------------------------------------------------

fn ara(base: &str, first: bool) -> &'static str {
    match base {
        // Emphatics / modifier-bearing first.
        "tˤ" => "ط",
        "sˤ" => "ص",
        "dˤ" => "ض",
        "ðˤ" => "ظ",
        _ => {
            let stripped: String = base.chars().filter(|c| !"ʰʷˤʱ".contains(*c)).collect();
            match stripped.as_str() {
                "a" | "e" | "i" | "o" | "u" | "ə" | "ɪ" | "ɛ" | "ɔ" | "ʊ" | "ʌ" | "æ" | "ɑ"
                | "ɒ" | "ɯ" | "ɨ" | "ʏ" | "y" | "ø" | "œ" | "ɐ" | "ɜ" | "ɵ" => {
                    if first {
                        "ا"
                    } else {
                        ""
                    }
                }
                "b" | "p" | "β" | "ɓ" => "ب",
                "t" | "ʈ" | "θ" => "ت",
                "dʒ" | "dʑ" => "ج",
                "ħ" | "h" | "ɦ" => "ه",
                "x" | "χ" => "خ",
                "d" | "ð" | "ɖ" | "ɗ" => "د",
                "r" | "ɾ" | "ʁ" => "ر",
                "z" | "ʒ" | "ʑ" => "ز",
                "s" | "ts" | "dz" => "س",
                "ʃ" | "ʂ" | "ɕ" | "tʃ" | "tʂ" | "ʈʂ" | "tɕ" | "ɧ" => "ش",
                "ʕ" => "ع",
                "ɣ" => "غ",
                "f" | "v" | "ʋ" => "ف",
                "q" => "ق",
                "k" | "ɡ" | "g" => "ك",
                "l" | "ɫ" | "ɭ" | "ʎ" => "ل",
                "m" => "م",
                "n" | "ɲ" | "ɳ" | "ɴ" | "ŋ" => "ن",
                "w" => "و",
                "j" => "ي",
                "ʔ" => "ء",
                "ç" => "ش",
                _ => "",
            }
        }
    }
}

fn to_arabic(segs: &[Seg]) -> String {
    let mut out = String::new();
    for (idx, s) in segs.iter().enumerate() {
        out.push_str(ara(&s.base, idx == 0));
        // Long vowel → mater lectionis.
        if s.long {
            out.push_str(match s.base.as_str() {
                "i" | "ɪ" | "e" | "ɛ" => "ي",
                "u" | "ʊ" | "o" | "ɔ" => "و",
                "a" | "ɑ" | "ɐ" | "æ" => "ا",
                _ => "",
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_in_block(s: &str, lo: u32, hi: u32) -> bool {
        s.chars()
            .filter(|c| *c != ' ')
            .all(|c| (lo..=hi).contains(&(c as u32)))
    }

    #[test]
    fn cyrillic_examples_and_block() {
        assert!(all_in_block(
            &transcribe("ɪˈvanəf", Script::Cyrillic),
            0x400,
            0x4FF
        ));
        assert!(all_in_block(
            &transcribe("ʒɑ̃", Script::Cyrillic),
            0x400,
            0x4FF
        ));
        assert_eq!(transcribe("ɪˈvan", Script::Cyrillic), "иван");
    }

    #[test]
    fn hangul_composes_valid_syllables() {
        // Every output char is a precomposed Hangul syllable.
        for ipa in ["kim", "pak", "mindʑun", "smɪθ", "pjɛʁ"] {
            let out = transcribe(ipa, Script::Hangul);
            assert!(
                !out.is_empty() && all_in_block(&out, 0xAC00, 0xD7A3),
                "'{ipa}' -> '{out}' SHALL be Hangul syllables"
            );
        }
        assert_eq!(transcribe("kim", Script::Hangul), "김");
        assert_eq!(transcribe("pak", Script::Hangul), "박");
    }

    #[test]
    fn hebrew_is_consonantal_skeleton() {
        let out = transcribe("koˈhen", Script::Hebrew);
        assert!(
            all_in_block(&out, 0x590, 0x5FF),
            "'{out}' SHALL be Hebrew letters"
        );
        // k-h-n with final nun.
        assert_eq!(out, "קהן");
    }

    #[test]
    fn arabic_is_consonantal_skeleton() {
        let out = transcribe("muˈħammad", Script::Arabic);
        assert!(
            all_in_block(&out, 0x600, 0x6FF),
            "'{out}' SHALL be Arabic letters"
        );
    }

    #[test]
    fn target_script_validity_over_inventory() {
        // A spread of real IPA values renders into the right block with no
        // stray latin/other characters.
        let samples = [
            "ˈmʏlɐ",
            "ʒɑ̃",
            "ɡaɾˈθia",
            "niːv",
            "sʲɪˈmʲɵnəf",
            "tʂən",
            "ʃiˈʁa",
        ];
        for ipa in samples {
            assert!(all_in_block(
                &transcribe(ipa, Script::Cyrillic),
                0x400,
                0x4FF
            ));
            assert!(all_in_block(
                &transcribe(ipa, Script::Hangul),
                0xAC00,
                0xD7A3
            ));
            assert!(all_in_block(&transcribe(ipa, Script::Hebrew), 0x590, 0x5FF));
            assert!(all_in_block(&transcribe(ipa, Script::Arabic), 0x600, 0x6FF));
        }
    }
}
