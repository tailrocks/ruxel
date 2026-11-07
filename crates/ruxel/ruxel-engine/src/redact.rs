use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{borrow::Cow, fmt};
use zeroize::Zeroizing;

/// Redacts secrets while keeping every stored pattern in zeroizing memory.
///
/// Matching scans the text so no non-zeroizing automaton retains copies of the
/// secret patterns.
pub struct Redactor {
    patterns: Vec<Zeroizing<String>>,
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
            patterns: Vec::new(),
        };
        this.rebuild(values);
        this
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    pub fn extend<'a>(&mut self, values: impl IntoIterator<Item = &'a str>) {
        self.patterns.extend(collect_patterns(values));
        dedup_patterns(&mut self.patterns);
    }

    #[must_use]
    pub fn redact<'s>(&self, s: &'s str) -> Cow<'s, str> {
        fallback_redact(s, &self.patterns)
    }

    fn rebuild<'a>(&mut self, values: impl IntoIterator<Item = &'a str>) {
        self.patterns = collect_patterns(values);
    }
}

fn collect_patterns<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<Zeroizing<String>> {
    let mut patterns = Vec::new();
    for value in values {
        if value.len() >= 6 {
            patterns.push(Zeroizing::new(value.to_owned()));
        }
        let encoded = Zeroizing::new(STANDARD.encode(value.as_bytes()));
        if encoded.len() >= 6 {
            patterns.push(Zeroizing::new(encoded.trim_end_matches('=').to_owned()));
            patterns.push(encoded);
        }
        if value.contains('\n') {
            patterns.extend(
                value
                    .lines()
                    .filter(|line| line.len() >= 6)
                    .map(|line| Zeroizing::new(line.to_owned())),
            );
        }
    }
    dedup_patterns(&mut patterns);
    patterns
}

fn dedup_patterns(patterns: &mut Vec<Zeroizing<String>>) {
    patterns.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    patterns.dedup_by(|a, b| a.as_str() == b.as_str());
}

fn fallback_redact<'s>(input: &'s str, patterns: &[Zeroizing<String>]) -> Cow<'s, str> {
    let mut rest = input;
    let mut output = String::new();
    loop {
        let next = patterns
            .iter()
            .filter_map(|pattern| {
                rest.find(pattern.as_str())
                    .map(|start| (start, pattern.len()))
            })
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
