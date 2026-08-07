use crate::git::GitData;
use crate::sources::CoAuthor;

pub struct RenderOptions {
    pub color: bool,
}

pub fn render_commit(git: &GitData, co_authors: &[CoAuthor], opts: &RenderOptions) -> String {
    let mut out = String::new();

    out.push_str(&format!("{} {}\n", word("commit", opts.color), git.hash));
    out.push_str(&format!(
        "{}: {} <{}>\n",
        word("Author", opts.color),
        git.author_name.as_deref().unwrap_or_default(),
        git.author_email.as_deref().unwrap_or_default()
    ));
    if let Some(date) = &git.date {
        // Three spaces after the colon, like git: the label column is padded to
        // 8 characters so the value lines up with "Author: ".
        out.push_str(&format!("{}:   {}\n", word("Date", opts.color), date));
    }
    out.push('\n');

    // Compose the body (message + trailers), then indent every line by four
    // spaces to match git's rendering of a commit message.
    let mut body = String::new();
    if !git.message.is_empty() {
        body.push_str(git.message.trim_end());
        body.push('\n');
        body.push('\n');
    }
    for ca in co_authors {
        // Generalizes the old "blank line before a co-author block" rule to any
        // entry; never double-blanks right after the message paragraph.
        if ca.blank_line_before && !body.ends_with("\n\n") {
            body.push('\n');
        }
        body.push_str(&format!(
            "{}: {} <{}>\n",
            word("Co-Authored-By", opts.color),
            ca.name,
            ca.email
        ));
    }
    for line in body.lines() {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }

    out
}

fn word(s: &str, color: bool) -> String {
    if color {
        use colored::Colorize;
        s.bold().cyan().to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::git::GitData;
    use crate::sources::CoAuthor;

    use super::{RenderOptions, render_commit};

    fn git() -> GitData {
        GitData {
            hash: "ab".repeat(20),
            message: "feat: test".to_string(),
            author_name: Some("Ann".to_string()),
            author_email: Some("ann@x.com".to_string()),
            date: Some("Thu Aug 6 05:32:10 2026 +0800".to_string()),
        }
    }

    fn git_without_date() -> GitData {
        GitData {
            date: None,
            ..git()
        }
    }

    fn trailers() -> Vec<CoAuthor> {
        vec![
            CoAuthor {
                name: "NixOS 26.11".to_string(),
                email: "os@system.invalid".to_string(),
                blank_line_before: false,
            },
            CoAuthor {
                name: "shiziku-laptop".to_string(),
                email: "shiziku-laptop@host.local".to_string(),
                blank_line_before: false,
            },
            CoAuthor {
                name: "Sample 5.0".to_string(),
                email: "noreply@example.com".to_string(),
                blank_line_before: true,
            },
        ]
    }

    fn opts() -> RenderOptions {
        RenderOptions { color: false }
    }

    #[test]
    fn header_and_message() {
        let out = render_commit(&git(), &[], &opts());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], format!("commit {}", "ab".repeat(20)));
        assert_eq!(lines[1], "Author: Ann <ann@x.com>");
        assert_eq!(lines[2], "Date:   Thu Aug 6 05:32:10 2026 +0800");
        assert!(lines[3].is_empty());
        assert_eq!(lines[4], "    feat: test");
    }

    #[test]
    fn no_date_line_when_date_missing() {
        let out = render_commit(&git_without_date(), &[], &opts());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[1], "Author: Ann <ann@x.com>");
        assert!(lines[2].is_empty(), "no Date line when date is None");
        assert_eq!(lines[3], "    feat: test");
    }

    #[test]
    fn trailers_in_config_order_with_blank_before_last() {
        let out = render_commit(&git(), &trailers(), &opts());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[6],
            "    Co-Authored-By: NixOS 26.11 <os@system.invalid>"
        );
        assert_eq!(
            lines[7],
            "    Co-Authored-By: shiziku-laptop <shiziku-laptop@host.local>"
        );
        assert_eq!(
            lines[8],
            "    ",
            "blank line before the blank_line_before trailer is indented like git"
        );
        assert_eq!(
            lines[9],
            "    Co-Authored-By: Sample 5.0 <noreply@example.com>"
        );
        assert_eq!(lines.len(), 10);
    }

    #[test]
    fn no_blank_line_before_means_no_trailing_blank() {
        let out = render_commit(&git(), &trailers()[..2], &opts());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            *lines.last().unwrap(),
            "    Co-Authored-By: shiziku-laptop <shiziku-laptop@host.local>"
        );
    }

    #[test]
    fn first_trailer_blank_line_before_is_noop() {
        let v = vec![CoAuthor {
            name: "Only".to_string(),
            email: "only@x.com".to_string(),
            blank_line_before: true,
        }];
        let out = render_commit(&git(), &v, &opts());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[5],
            "    ",
            "message paragraph keeps its single blank line"
        );
        assert_eq!(lines[6], "    Co-Authored-By: Only <only@x.com>");
        assert_eq!(lines.len(), 7, "no second blank before the first trailer");
    }
}
