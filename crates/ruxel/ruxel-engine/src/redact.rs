use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE},
};
use std::{borrow::Cow, fmt};

pub struct Redactor {
    matcher: Option<AhoCorasick>,
    patterns: Vec<String>,
}

impl fmt::Debug for Redactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Redactor")
            .field("pattern_count", &self.patterns.len())
            .finish_non_exhaustive()
    }
}

impl Redactor {
    #[must_use]
    pub fn new<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let mut this = Self {
            matcher: None,
            patterns: Vec::new(),
        };
        this.rebuild(values);
        this
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            matcher: None,
            patterns: Vec::new(),
        }
    }

    pub fn extend<'a>(&mut self, values: impl IntoIterator<Item = &'a str>) {
        self.patterns.extend(collect_patterns(values));
        self.patterns.sort();
        self.patterns.dedup();
        self.matcher = build(self.patterns.clone());
    }

    #[must_use]
    pub fn redact<'s>(&self, s: &'s str) -> Cow<'s, str> {
        if let Some(matcher) = &self.matcher {
            if !matcher.is_match(s) {
                return Cow::Borrowed(s);
            }
            return Cow::Owned(matcher.replace_all(s, &vec!["[REDACTED]"; self.patterns.len()]));
        }
        fallback_redact(s, &self.patterns)
    }

    fn rebuild<'a>(&mut self, values: impl IntoIterator<Item = &'a str>) {
        self.patterns = collect_patterns(values);
        self.matcher = build(self.patterns.clone());
    }
}

fn collect_patterns<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut patterns = Vec::new();
    for value in values {
        if value.len() >= 6 {
            patterns.push(value.to_owned());
        }
        let bytes = value.as_bytes();
        // Redaction covers the exact value and its standard and URL-safe
        // base64 and hex forms: a secret that passes through an encoding
        // step in the workflow is redacted in that shape too.
        for encoded in [STANDARD.encode(bytes), URL_SAFE.encode(bytes)] {
            if encoded.len() >= 6 {
                patterns.push(encoded.trim_end_matches('=').to_owned());
                patterns.push(encoded);
            }
        }
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            hex.push(char::from(HEX[usize::from(byte >> 4)]));
            hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        if hex.len() >= 6 {
            patterns.push(hex);
        }
        if value.contains('\n') {
            patterns.extend(
                value
                    .lines()
                    .filter(|line| line.len() >= 6)
                    .map(str::to_owned),
            );
        }
    }
    patterns.sort();
    patterns.dedup();
    patterns
}

fn build(patterns: Vec<String>) -> Option<AhoCorasick> {
    if patterns.is_empty() {
        None
    } else {
        AhoCorasickBuilder::new()
            .match_kind(MatchKind::LeftmostLongest)
            .build(patterns)
            .ok()
    }
}

fn fallback_redact<'s>(input: &'s str, patterns: &[String]) -> Cow<'s, str> {
    let mut rest = input;
    let mut output = String::new();
    loop {
        let next = patterns
            .iter()
            .filter_map(|pattern| rest.find(pattern).map(|start| (start, pattern.len())))
            .min_by(|(a_start, a_len), (b_start, b_len)| {
                a_start.cmp(b_start).then_with(|| b_len.cmp(a_len))
            });
        let Some((start, length)) = next else {
            if output.is_empty() {
                return Cow::Borrowed(input);
            }
            output.push_str(rest);
            return Cow::Owned(output);
        };
        output.push_str(&rest[..start]);
        output.push_str("[REDACTED]");
        rest = &rest[start + length..];
    }
}

#[cfg(test)]
#[path = "redact/tests.rs"]
mod tests;
