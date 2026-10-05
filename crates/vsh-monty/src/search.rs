//! Literal matching over already authorized VFS content, never a host index.
use memchr::memmem::Finder;

pub(super) struct LiteralSearch<'a> {
    finder: Finder<'a>,
    single_line: bool,
}

impl<'a> LiteralSearch<'a> {
    pub(super) fn new(query: &'a str) -> Self {
        Self {
            finder: Finder::new(query.as_bytes()),
            single_line: !query.contains('\n'),
        }
    }

    pub(super) fn visit(&self, text: &str, mut visitor: impl FnMut(usize, &str) -> bool) {
        if !self.single_line {
            return;
        }
        let bytes = text.as_bytes();
        let mut cursor = 0;
        let mut number = 1;
        while cursor < bytes.len() {
            let Some(offset) = self.finder.find(&bytes[cursor..]) else {
                break;
            };
            let hit = cursor + offset;
            let skipped = &bytes[cursor..hit];
            number += memchr::memchr_iter(b'\n', skipped).count();
            let start = memchr::memrchr(b'\n', skipped).map_or(cursor, |last| cursor + last + 1);
            let newline = memchr::memchr(b'\n', &bytes[hit..]).map(|at| hit + at);
            let end = newline.unwrap_or(bytes.len());
            // ASCII newline boundaries are also UTF-8 boundaries. Match str::lines:
            // strip CR only in CRLF, retaining a bare CR at end-of-file.
            let line = &text[start..end];
            let line = if newline.is_some() {
                line.strip_suffix('\r').unwrap_or(line)
            } else {
                line
            };
            if !visitor(number, line) {
                break;
            }
            let Some(end) = newline else {
                break;
            };
            cursor = end + 1;
            number += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_preserve_line_semantics() {
        for text in [
            "",
            "one\none\n",
            "é東京 one\r\nlast\r",
            "one\rone",
            "\n\r\n",
            "one\n\nlast",
            "miss\nmiss\none one\nmiss\none",
            "\0one\n東京\0one\r\n",
        ] {
            for query in ["one", "é", "東京", "\r", "\n", "one\n", "last", "[literal]"] {
                let mut found = Vec::new();
                LiteralSearch::new(query).visit(text, |number, line| {
                    if line.contains(query) {
                        found.push((number, line.to_owned()));
                    }
                    true
                });
                let expected: Vec<_> = text
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.contains(query))
                    .map(|(i, line)| (i + 1, line.to_owned()))
                    .collect();
                assert_eq!(found, expected, "text={text:?} query={query:?}");
            }
        }
    }

    #[test]
    fn candidates_match_lines_oracle_for_composed_inputs() {
        let atoms = ["a", "aa", "é", "東京", "\r", "\n", "\0", ""];
        let queries = [
            "a", "aa", "aaa", "é", "東京", "\r", "\n", "\r\n", "\0", "a\r", "absent",
        ];
        for first in atoms {
            for second in atoms {
                for third in atoms {
                    for fourth in atoms {
                        let text = [first, second, third, fourth].concat();
                        for query in queries {
                            let mut found = Vec::new();
                            LiteralSearch::new(query).visit(&text, |number, line| {
                                if line.contains(query) {
                                    found.push((number, line.to_owned()));
                                }
                                true
                            });
                            let expected: Vec<_> = text
                                .lines()
                                .enumerate()
                                .filter(|(_, line)| line.contains(query))
                                .map(|(i, line)| (i + 1, line.to_owned()))
                                .collect();
                            assert_eq!(found, expected, "text={text:?} query={query:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn visitor_stops_at_first_matching_line() {
        let mut count = 0;
        LiteralSearch::new("one").visit("miss\none one\none\none", |number, line| {
            count += 1;
            assert_eq!((number, line), (2, "one one"));
            false
        });
        assert_eq!(count, 1);
    }
}
