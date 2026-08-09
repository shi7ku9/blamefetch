use std::collections::HashMap;

pub fn render(template: &str, fields: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                if bytes.get(i + 1) == Some(&b'{') {
                    // `{{` escapes a literal `{`.
                    out.push('{');
                    i += 2;
                } else if let Some(rel) = template[i + 1..].find('}') {
                    let key = &template[i + 1..i + 1 + rel];
                    if key.is_empty() {
                        // `{}` — an empty placeholder is not a placeholder;
                        // keep it literal so bare braces can be printed.
                        out.push_str("{}");
                    } else if let Some(val) = fields.get(key) {
                        out.push_str(val);
                    }
                    i += 1 + rel + 1;
                } else {
                    // Unclosed `{` — print the rest verbatim.
                    out.push_str(&template[i..]);
                    break;
                }
            }
            b'}' => {
                if bytes.get(i + 1) == Some(&b'}') {
                    // `}}` escapes a literal `}`.
                    out.push('}');
                    i += 2;
                } else {
                    out.push('}');
                    i += 1;
                }
            }
            _ => {
                // Skip to the next brace in one jump (byte index is safe:
                // braces are ASCII, so the found position is a char boundary;
                // searching from `i` keeps the start on a boundary even when
                // the current character is multibyte UTF-8).
                let next = template[i..]
                    .find(['{', '}'])
                    .map(|r| i + r)
                    .unwrap_or(template.len());
                out.push_str(&template[i..next]);
                i = next;
            }
        }
    }
    collapse_whitespace(&out)
}

pub fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{collapse_whitespace, render};

    #[test]
    fn replaces_known_fields() {
        let mut f = HashMap::new();
        f.insert("version".to_string(), "5.0".to_string());
        assert_eq!(render("Sample {version}", &f), "Sample 5.0");
    }

    #[test]
    fn unknown_fields_vanish_and_whitespace_collapses() {
        let f = HashMap::new();
        assert_eq!(render("a {missing} b", &f), "a b");
    }

    #[test]
    fn multiple_fields_and_literal_text() {
        let mut f = HashMap::new();
        f.insert("model".to_string(), "Intel i7".to_string());
        f.insert("cores".to_string(), "8".to_string());
        assert_eq!(
            render("{model} ({cores} threads)", &f),
            "Intel i7 (8 threads)"
        );
    }

    #[test]
    fn no_braces_is_identity() {
        assert_eq!(render("hello world", &HashMap::new()), "hello world");
    }

    #[test]
    fn empty_placeholder_is_kept_literal() {
        let f = HashMap::new();
        assert_eq!(render("a{}b", &f), "a{}b");
        assert_eq!(render("{}", &f), "{}");
        assert_eq!(render("{} {x}", &f), "{}");
    }

    #[test]
    fn doubled_braces_are_literal_escapes() {
        let f = HashMap::new();
        assert_eq!(render("{{}}", &f), "{}");
        assert_eq!(render("a {{ b }} c", &f), "a { b } c");
        assert_eq!(render("pre }} post", &f), "pre } post");
    }

    #[test]
    fn escaped_placeholder_is_not_substituted() {
        let mut f = HashMap::new();
        f.insert("bot_version".to_string(), "2.1.233".to_string());
        assert_eq!(render("{bot_version}", &f), "2.1.233");
        assert_eq!(render("{{bot_version}}", &f), "{bot_version}");
    }

    #[test]
    fn utf8_text_is_not_split_at_byte_boundaries() {
        let mut f = HashMap::new();
        f.insert("rank".to_string(), "SSS".to_string());
        assert_eq!(
            render("私は猫娘 {rank}", &f),
            "私は猫娘 SSS",
            "multibyte text before a placeholder must render verbatim"
        );
        assert_eq!(
            render("にゃん！私は猫娘、尻尾ふりふり參上だよ！", &HashMap::new()),
            "にゃん！私は猫娘、尻尾ふりふり參上だよ！",
            "plain UTF-8 text without braces must render verbatim"
        );
    }

    #[test]
    fn collapse_whitespace_squashes_spaces_tabs_newlines() {
        assert_eq!(collapse_whitespace("a  b\n\tc"), "a b c");
        assert_eq!(collapse_whitespace("  only  "), "only");
    }
}
