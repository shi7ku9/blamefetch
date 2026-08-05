use std::collections::HashMap;

pub fn render(template: &str, fields: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end_rel) => {
                if end_rel > 0 {
                    let key = &rest[start + 1..start + end_rel];
                    if let Some(val) = fields.get(key) {
                        out.push_str(val);
                    }
                }
                rest = &rest[start + end_rel + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
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
    fn empty_placeholder_is_skipped_without_panicking() {
        let f = HashMap::new();
        assert_eq!(render("a{}b", &f), "ab");
        assert_eq!(render("{}", &f), "");
    }

    #[test]
    fn collapse_whitespace_squashes_spaces_tabs_newlines() {
        assert_eq!(collapse_whitespace("a  b\n\tc"), "a b c");
        assert_eq!(collapse_whitespace("  only  "), "only");
    }
}
