//! Minimal BibTeX reader/writer for astrobib's narrow needs.
//!
//! Files are machine-written (by ADS export or by astrobib itself), one
//! entry per file, every value brace-delimited — so this is a tolerant
//! hand parser, not a general BibTeX implementation. Field names are
//! lowercased on parse (as bibtexparser does); ID and ENTRYTYPE are the
//! same synthetic uppercase keys the Python side uses. LaTeX accent
//! macros are converted to Unicode (the convert_to_unicode analogue);
//! other braces are preserved verbatim, matching the Python data model.

use indexmap::IndexMap;

/// Entry data: insertion-ordered, because serialization writes fields
/// outside FIELD_ORDER in insertion order, exactly like a Python dict.
pub type Data = IndexMap<String, String>;

/// Field order for serialization — must match library.py FIELD_ORDER so
/// both implementations write identical files for identical data.
/// Note the camelCase names never match parsed data (bibtexparser
/// lowercases field names and the lookup is case-sensitive, in Python
/// too) — parsed archiveprefix/primaryclass land in the trailing
/// insertion-ordered section. That quirk is the on-disk format; keep it.
pub const FIELD_ORDER: &[&str] = &[
    "author",
    "title",
    "year",
    "journal",
    "booktitle",
    "volume",
    "number",
    "pages",
    "month",
    "publisher",
    "eprint",
    "archivePrefix",
    "primaryClass",
    "doi",
    "adsurl",
    "adsnote",
    "keywords",
    "abstract",
    "astrobib_starred",
];

/// Parse the first entry in a .bib file into the flat data map the rest of
/// the code operates on. Returns None on files with no parseable entry.
pub fn parse_entry(text: &str) -> Option<Data> {
    parse_one(text, 0).map(|(d, _)| d)
}

/// Parse every entry in a (possibly multi-entry) .bib text, in order.
pub fn parse_entries(text: &str) -> Vec<Data> {
    let mut out = vec![];
    let mut pos = 0;
    while let Some((d, end)) = parse_one(text, pos) {
        out.push(d);
        pos = end;
    }
    out
}

/// Parse one entry starting at or after `from`; returns the data map and
/// the index just past the entry's closing brace.
fn parse_one(text: &str, from: usize) -> Option<(Data, usize)> {
    let bytes = text.as_bytes();
    let at = text[from..].find('@')? + from;
    let brace = text[at..].find('{')? + at;
    let etype = text[at + 1..brace].trim().to_lowercase();
    // @comment/@string/@preamble blocks are skipped, not entries
    if matches!(etype.as_str(), "comment" | "string" | "preamble") {
        let (_, end) = scan_braced(text, brace);
        return parse_one(text, end);
    }
    let mut i = brace + 1;

    let key_end = text[i..].find(',')? + i;
    let key = text[i..key_end].trim().to_string();
    i = key_end + 1;

    let mut fields: Vec<(String, String)> = vec![];

    loop {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'}' {
            break;
        }
        let eq = match text[i..].find('=') {
            Some(off) => i + off,
            None => break,
        };
        let name = text[i..eq].trim().to_lowercase();
        i = eq + 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let raw = match bytes[i] {
            b'{' => {
                let (val, end) = scan_braced(text, i);
                i = end;
                val
            }
            b'"' => {
                let end = text[i + 1..].find('"').map(|o| i + 1 + o).unwrap_or(bytes.len());
                let val = text[i + 1..end].to_string();
                i = (end + 1).min(bytes.len());
                val
            }
            _ => {
                let end = text[i..]
                    .find([',', '}'])
                    .map(|o| i + o)
                    .unwrap_or(bytes.len());
                let val = text[i..end].trim();
                i = end;
                // bare values are @string macros: expand the common ones
                // (BibTexParser(common_strings=True) month names)
                expand_common_string(val).to_string()
            }
        };
        if !name.is_empty() {
            fields.push((name, latex_to_unicode(&raw)));
        }
    }
    let end = (i + 1).min(bytes.len());
    // bibtexparser v1 stores fields in reverse file order with
    // ENTRYTYPE/ID appended last; serialization writes unknown fields in
    // dict order, so reproduce the layout exactly or rewritten files
    // shuffle their trailing fields vs. the Python tool.
    let mut data = Data::new();
    for (name, value) in fields.into_iter().rev() {
        data.insert(name, value);
    }
    data.insert("ENTRYTYPE".to_string(), etype);
    data.insert("ID".to_string(), key);
    Some((data, end))
}

/// Scan a balanced-brace value starting at the opening brace; returns the
/// inner text and the index just past the closing brace. Backslash escapes
/// the following character.
fn scan_braced(text: &str, start: usize) -> (String, usize) {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = start;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if escaped {
            escaped = false;
        } else if b == b'\\' {
            escaped = true;
        } else if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return (text[start + 1..i].to_string(), i + 1);
            }
        }
        i += 1;
    }
    (text[start + 1..].to_string(), bytes.len())
}

/// Combining diacritic for a LaTeX accent command character.
fn combining(accent: char) -> Option<char> {
    Some(match accent {
        '\'' => '\u{0301}',
        '`' => '\u{0300}',
        '"' => '\u{0308}',
        '^' => '\u{0302}',
        '~' => '\u{0303}',
        '=' => '\u{0304}',
        '.' => '\u{0307}',
        'u' => '\u{0306}',
        'v' => '\u{030C}',
        'c' => '\u{0327}',
        'H' => '\u{030B}',
        'k' => '\u{0328}',
        'r' => '\u{030A}',
        'b' => '\u{0331}',
        'd' => '\u{0323}',
        _ => return None,
    })
}

/// bibtexparser's COMMON_STRINGS: month macro → full name.
fn expand_common_string(name: &str) -> &str {
    match name {
        "jan" => "January",
        "feb" => "February",
        "mar" => "March",
        "apr" => "April",
        "may" => "May",
        "jun" => "June",
        "jul" => "July",
        "aug" => "August",
        "sep" => "September",
        "oct" => "October",
        "nov" => "November",
        "dec" => "December",
        other => other,
    }
}

/// Standalone letter macros (\o, \aa, \ss, …) and escaped specials.
fn simple_macro(name: &str) -> Option<&'static str> {
    Some(match name {
        "o" => "ø",
        "O" => "Ø",
        "l" => "ł",
        "L" => "Ł",
        "aa" => "å",
        "AA" => "Å",
        "ae" => "æ",
        "AE" => "Æ",
        "oe" => "œ",
        "OE" => "Œ",
        "ss" => "ß",
        "i" => "ı",
        "&" => "&",
        "%" => "%",
        "$" => "$",
        "#" => "#",
        "_" => "_",
        "{" => "{",
        "}" => "}",
        // Common math-mode symbols from ADS titles (usually inside
        // \ensuremath{...} wrappers). Where bibtexparser v1 converted the
        // macro at all we match its output codepoints (\sim → U+223C,
        // \mu → U+03BC, \pm → U+00B1, Greek letters); \times, \deg and
        // \odot it either left literal or mangled (\odot → "ødot"), so
        // these are display improvements over the Python behavior.
        "sim" => "∼",
        "mu" => "μ",
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "times" => "×",
        "pm" => "±",
        "deg" => "°",
        "odot" => "⊙",
        _ => return None,
    })
}

/// Conversion of LaTeX accent/letter macros to Unicode, normalized to
/// NFC, with every remaining brace removed afterward — matching
/// bibtexparser.latexenc.latex_to_unicode, which strips all braces
/// (grouping and protective alike, and even escaped ones, since \{
/// converts to a literal brace before the strip).
pub fn latex_to_unicode(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        // {\'e} / {\'{e}} / \'e / \'{e} → é ; {\o} / \o → ø
        let (open, j) = if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] == '\\' {
            (true, i + 1)
        } else if chars[i] == '\\' {
            (false, i)
        } else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        if let Some((converted, next)) = convert_macro(&chars, j, open) {
            out.push_str(&converted);
            i = next;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    let out: String = out.chars().filter(|&c| c != '{' && c != '}').collect();
    out.nfc().collect()
}

/// Try to convert one macro at `chars[bs]` (a backslash); `wrapped` means a
/// surrounding {...} group whose closing brace should be consumed too.
fn convert_macro(chars: &[char], bs: usize, wrapped: bool) -> Option<(String, usize)> {
    let mut i = bs + 1;
    if i >= chars.len() {
        return None;
    }
    let c = chars[i];
    if c.is_ascii_alphabetic() {
        // alphabetic command word
        let start = i;
        while i < chars.len() && chars[i].is_ascii_alphabetic() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        // \ensuremath{...} is a no-op wrapper: drop the macro name and let
        // the main loop convert the braced argument (stray braces are
        // stripped at the end anyway). A bare \ensuremath with no braced
        // argument stays literal, matching bibtexparser v1 and the golden
        // vectors ("Binary\ensuremath-disk").
        if word == "ensuremath" && i < chars.len() && chars[i] == '{' {
            return Some((String::new(), i));
        }
        if let Some(rep) = simple_macro(&word) {
            let end = finish_group(chars, i, wrapped)?;
            return Some((rep.to_string(), end));
        }
        // single-letter accent commands taking an argument: \c{c} \v{s} …
        if word.len() == 1 {
            if let Some(mark) = combining(word.chars().next().unwrap()) {
                let (letter, after) = accent_target(chars, i)?;
                let end = finish_group(chars, after, wrapped)?;
                return Some((format!("{letter}{mark}"), end));
            }
        }
        None
    } else {
        // symbol command: accent (\'e) or escaped special (\&)
        if let Some(rep) = simple_macro(&c.to_string()) {
            let end = finish_group(chars, i + 1, wrapped)?;
            return Some((rep.to_string(), end));
        }
        let mark = combining(c)?;
        let (letter, after) = accent_target(chars, i + 1)?;
        let end = finish_group(chars, after, wrapped)?;
        Some((format!("{letter}{mark}"), end))
    }
}

/// The letter an accent applies to: `e`, `{e}`, or `\i`.
fn accent_target(chars: &[char], mut i: usize) -> Option<(char, usize)> {
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    if chars[i] == '{' {
        if i + 2 < chars.len() && chars[i + 2] == '}' && chars[i + 1].is_alphabetic() {
            return Some((chars[i + 1], i + 3));
        }
        if i + 3 < chars.len() && chars[i + 1] == '\\' && chars[i + 2] == 'i' && chars[i + 3] == '}' {
            return Some(('ı', i + 4));
        }
        return None;
    }
    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == 'i' {
        return Some(('ı', i + 2));
    }
    if chars[i].is_alphabetic() {
        return Some((chars[i], i + 1));
    }
    None
}

/// Consume the closing brace of a wrapped macro group, if any.
fn finish_group(chars: &[char], i: usize, wrapped: bool) -> Option<usize> {
    if !wrapped {
        return Some(i);
    }
    if i < chars.len() && chars[i] == '}' {
        Some(i + 1)
    } else {
        None
    }
}

/// Serialize an entry — byte-for-byte port of library.py format_bib_entry:
/// FIELD_ORDER first (case-sensitive match), then everything else in
/// insertion order.
pub fn format_entry(data: &Data) -> String {
    let key = data.get("ID").map(String::as_str).unwrap_or("");
    let etype = data.get("ENTRYTYPE").map(String::as_str).unwrap_or("article");
    let mut lines = vec![format!("@{etype}{{{key},")];
    let mut seen: Vec<&str> = vec!["ID", "ENTRYTYPE"];
    for name in FIELD_ORDER {
        if let Some(val) = data.get(*name) {
            lines.push(format!("  {name:<16} = {{{val}}},"));
            seen.push(name);
        }
    }
    for (name, val) in data {
        if !seen.contains(&name.as_str()) {
            lines.push(format!("  {name:<16} = {{{val}}},"));
        }
    }
    lines.push("}\n".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ads_style_entry() {
        let text = r#"@ARTICLE{Zrake2019abcde,
  author           = {{Zrake}, Jonathan and {MacFadyen}, Andrew},
  title            = {{Magnetic energy production}},
  year             = {2019},
  adsurl           = {https://ui.adsabs.harvard.edu/abs/2019ApJ...123..456Z},
}
"#;
        let d = parse_entry(text).unwrap();
        assert_eq!(d["ID"], "Zrake2019abcde");
        assert_eq!(d["ENTRYTYPE"], "article");
        // braces are stripped on parse, as bibtexparser's convert_to_unicode does
        assert_eq!(d["author"], "Zrake, Jonathan and MacFadyen, Andrew");
        assert_eq!(d["title"], "Magnetic energy production");
        assert_eq!(d["year"], "2019");
    }

    #[test]
    fn converts_accents() {
        assert_eq!(latex_to_unicode(r"{\'e}"), "é");
        assert_eq!(latex_to_unicode(r"\'e"), "é");
        assert_eq!(latex_to_unicode(r"\'{e}"), "é");
        assert_eq!(latex_to_unicode(r#"{\"u}"#), "ü");
        assert_eq!(latex_to_unicode(r"\c{c}"), "ç");
        assert_eq!(latex_to_unicode(r"\v{s}"), "š");
        assert_eq!(latex_to_unicode(r"{\o}"), "ø");
        assert_eq!(latex_to_unicode(r"\&"), "&");
        assert_eq!(latex_to_unicode(r"M{\'u}noz"), "Múnoz");
        assert_eq!(latex_to_unicode("{Title Case}"), "Title Case");
        assert_eq!(latex_to_unicode(r"{{Double} braced}"), "Double braced");
    }

    #[test]
    fn converts_ensuremath() {
        // wrapper stripped, inner macro converted
        assert_eq!(latex_to_unicode(r"{\ensuremath{\sim}}100"), "∼100");
        assert_eq!(latex_to_unicode(r"\ensuremath{\mu}m"), "μm");
        assert_eq!(latex_to_unicode(r"\ensuremath{\alpha}"), "α");
        assert_eq!(latex_to_unicode(r"\ensuremath{\times}"), "×");
        assert_eq!(latex_to_unicode(r"M\ensuremath{_\odot}"), "M_⊙");
        assert_eq!(
            latex_to_unicode(r"\ensuremath{\sim}5 \ensuremath{\pm} 2 \ensuremath{\deg}"),
            "∼5 ± 2 °"
        );
        // non-macro argument: wrapper still stripped
        assert_eq!(latex_to_unicode(r"10\ensuremath{^{-3}}"), "10^-3");
        // unknown inner macro degrades to the usual unknown-macro fallback,
        // never the literal string "\ensuremath"
        assert_eq!(latex_to_unicode(r"\ensuremath{\foo}"), r"\foo");
        // bare \ensuremath without a braced argument stays literal, as in
        // bibtexparser v1 (and the golden format vector)
        assert_eq!(
            latex_to_unicode(r"Binary\ensuremath-disk"),
            r"Binary\ensuremath-disk"
        );
    }

    #[test]
    fn converts_math_macros() {
        assert_eq!(latex_to_unicode(r"\sim"), "∼");
        assert_eq!(latex_to_unicode(r"{\mu}Jy"), "μJy");
        assert_eq!(latex_to_unicode(r"\beta"), "β");
        assert_eq!(latex_to_unicode(r"\gamma"), "γ");
        // longer words are not prefix-matched
        assert_eq!(latex_to_unicode(r"\mum"), r"\mum");
    }

    #[test]
    fn parses_multiple_entries() {
        let text = r#"@string{apj = "The Astrophysical Journal"}
@ARTICLE{A2020aaaaa,
  author = {A, X.},
  year = {2020},
}
% comment line
@ARTICLE{B2021bbbbb,
  author = {B, Y.},
  year = {2021},
}
"#;
        let all = parse_entries(text);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["ID"], "A2020aaaaa");
        assert_eq!(all[1]["ID"], "B2021bbbbb");
    }

    #[test]
    fn roundtrips_format() {
        let mut d = Data::new();
        d.insert("ID".into(), "Key2020abcde".into());
        d.insert("ENTRYTYPE".into(), "article".into());
        d.insert("author".into(), "Someone, A.".into());
        d.insert("year".into(), "2020".into());
        let text = format_entry(&d);
        let back = parse_entry(&text).unwrap();
        assert_eq!(back["ID"], "Key2020abcde");
        assert_eq!(back["author"], "Someone, A.");
    }
}
