//! Rewrites the store's SQLite-flavored SQL into what PostgreSQL parses.
//!
//! Only two things differ after the store's statements were moved to the
//! common subset: placeholders (`?` and `?N` become `$N`) and `LIKE`, which is
//! case-insensitive for ASCII in SQLite and maps to `ILIKE`.

/// Translates one statement. String literals, quoted identifiers and line
/// comments are copied through untouched.
pub fn translate(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len() + 8);
    let mut next_param = 1_usize;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\'' | b'"' => {
                let end = quoted_end(bytes, i);
                out.push_str(&sql[i..end]);
                i = end;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                let end = bytes[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(bytes.len(), |p| i + p);
                out.push_str(&sql[i..end]);
                i = end;
            }
            b'?' => {
                let digits = bytes[i + 1..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                out.push('$');
                if digits == 0 {
                    out.push_str(&next_param.to_string());
                    next_param += 1;
                } else {
                    out.push_str(&sql[i + 1..i + 1 + digits]);
                }
                i += 1 + digits;
            }
            _ if is_word(c) => {
                let len = bytes[i..].iter().take_while(|&&b| is_word(b)).count();
                let word = &sql[i..i + len];
                if word.eq_ignore_ascii_case("like") {
                    out.push_str("ILIKE");
                } else {
                    out.push_str(word);
                }
                i += len;
            }
            _ => {
                // Multi-byte characters only occur inside literals, which the
                // quoted arm copies whole, so a byte here is one ASCII char.
                out.push(c as char);
                i += 1;
            }
        }
    }
    out
}

const fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The index just past the literal or identifier opening at `start`. A doubled
/// quote inside is an escaped quote, not the end.
fn quoted_end(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::translate;

    #[test]
    fn numbers_bare_placeholders_in_order() {
        assert_eq!(
            translate("SELECT a FROM t WHERE b = ? AND c = ?"),
            "SELECT a FROM t WHERE b = $1 AND c = $2"
        );
    }

    #[test]
    fn keeps_explicit_placeholder_numbers() {
        assert_eq!(
            translate("INSERT INTO t(a, b) VALUES(?1, ?2) ON CONFLICT DO UPDATE SET b = ?2"),
            "INSERT INTO t(a, b) VALUES($1, $2) ON CONFLICT DO UPDATE SET b = $2"
        );
    }

    #[test]
    fn leaves_literals_identifiers_and_comments_alone() {
        assert_eq!(
            translate("SELECT '?', \"a?b\", 'it''s like ?' -- why?\nFROM t WHERE x = ?"),
            "SELECT '?', \"a?b\", 'it''s like ?' -- why?\nFROM t WHERE x = $1"
        );
    }

    #[test]
    fn like_becomes_ilike_as_a_whole_word_only() {
        assert_eq!(
            translate("WHERE name LIKE ? ESCAPE '\\' AND likes NOT like ?"),
            "WHERE name ILIKE $1 ESCAPE '\\' AND likes NOT ILIKE $2"
        );
    }

    #[test]
    fn non_ascii_literals_survive() {
        assert_eq!(
            translate("SELECT '한글' WHERE a = ?"),
            "SELECT '한글' WHERE a = $1"
        );
    }
}
