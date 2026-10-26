//! The policy library: pure functions over the text of a source and the passages that rules cite. No I/O.
//!
//! Standard: OSCAL 1.1.2 catalog and back-matter hold the shape. W3C Web Annotation's text quote selector is the model
//! for a passage copied word for word.

/// The text a source says, as one line of words. `text/plain` is decoded as UTF-8. `text/html` loses its scripts, styles
/// and tags and has its entities decoded. In both, white space is one space and the ends are trimmed.
///
/// `ponytail:` this is a tag stripper, not an HTML parser. A page that builds its text with scripts yields nothing useful,
/// and the source must be added as `text`.
pub fn normalize_text(media_type: &str, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let is_html = media_type.trim().to_ascii_lowercase().starts_with("text/html");
    if is_html {
        collapse(&decode_entities(&strip_tags(&drop_blocks(&text))))
    } else {
        collapse(&text)
    }
}

/// One space between words, none at the ends.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Removes everything from each `<script` to its `</script>`, and the same for `<style`. A block that never closes runs to the end.
fn drop_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while at < html.len() {
        let next = ["script", "style"].iter().filter_map(|tag| lower[at..].find(&format!("<{tag}")).map(|i| (at + i, *tag))).min();
        let Some((start, tag)) = next else {
            out.push_str(&html[at..]);
            break;
        };
        out.push_str(&html[at..start]);
        let close = format!("</{tag}>");
        match lower[start..].find(&close) {
            Some(i) => at = start + i + close.len(),
            None => break,
        }
        out.push(' ');
    }
    out
}

/// Removes every tag and puts a space where it was. A `>` inside a quoted attribute does not end the tag.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut quote: Option<char> = None;
    let mut in_tag = false;
    for c in html.chars() {
        if in_tag {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(c),
                (None, '>') => {
                    in_tag = false;
                    out.push(' ');
                }
                _ => {}
            }
        } else if c == '<' {
            in_tag = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Decodes `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;` and `&nbsp;`, each once, left to right.
fn decode_entities(text: &str) -> String {
    const ENTITIES: [(&str, &str); 6] = [("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&nbsp;", "\u{a0}")];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        for (entity, plain) in ENTITIES {
            if let Some(after) = rest.strip_prefix(entity) {
                out.push_str(plain);
                rest = after;
                continue 'scan;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

/// Whether `passage`, with white space collapsed as `normalize_text` does, occurs in `text`. A passage shorter than 20
/// characters or longer than 2,000 is never in it.
pub fn passage_in(text: &str, passage: &str) -> bool {
    let passage = collapse(passage);
    let length = passage.chars().count();
    (20..=2000).contains(&length) && collapse(text).contains(&passage)
}

/// The `radius` characters of `text` on each side of `passage`, for a reviewer to read it in context. `None` when the
/// passage is not in the text. White space is collapsed as `passage_in` does.
pub fn context_around(text: &str, passage: &str, radius: usize) -> Option<(String, String)> {
    let text = collapse(text);
    let passage = collapse(passage);
    let start = text.find(&passage)?;
    let before: Vec<char> = text[..start].chars().rev().take(radius).collect();
    let after = text[start + passage.len()..].chars().take(radius).collect();
    Some((before.into_iter().rev().collect(), after))
}

/// Whether a binding's target is there and in use. The caller says which targets are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    Enforced,
    Missing,
}

pub fn binding_state(target_kind: &str, target_ref: &str, lookup: &dyn Fn(&str, &str) -> bool) -> BindingState {
    if lookup(target_kind, target_ref) {
        BindingState::Enforced
    } else {
        BindingState::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_around_gives_the_characters_on_each_side_and_none_when_the_passage_is_absent() {
        let text = format!("{} Needle sentence that is long enough here. {}", "a".repeat(400), "b".repeat(400));
        let (before, after) = context_around(&text, "Needle sentence that is\nlong enough here.", 300).unwrap();
        assert_eq!(before.chars().count(), 300);
        assert!(before.ends_with(' ') && before.starts_with('a'));
        assert_eq!(after.chars().count(), 300);
        let (b, _) = context_around("short lead. Needle sentence that is long enough here.", "Needle sentence that is long enough here.", 300).unwrap();
        assert_eq!(b, "short lead. ", "less than the radius when the text starts sooner");
        assert!(context_around(&text, "Needle sentence that is long enough there.", 300).is_none());
    }

    const PAGE: &str = "<html><head><title>Records</title><style>body { color: red; }</style>\n<script>var x = '<b>not text</b>';\nalert(1)</script></head>\n<body>\n  <h1>Records   retention</h1>\n<p>Student records &amp; grades are kept for\n seven years.&nbsp;After that, they are&nbsp;destroyed.</p>\n<p>&quot;Quoted&quot; and &lt;marked&gt; &#39;text&#39; stay.</p></body></html>";

    #[test]
    fn html_gives_the_visible_sentences_once_in_order() {
        let text = normalize_text("text/html", PAGE.as_bytes());
        assert_eq!(text, "Records Records retention Student records & grades are kept for seven years. After that, they are destroyed. \"Quoted\" and <marked> 'text' stay.");
        assert!(!text.contains("alert") && !text.contains("color: red") && !text.contains("<b>"));
    }

    #[test]
    fn plain_text_is_decoded_and_collapsed_and_bad_bytes_are_replaced() {
        assert_eq!(normalize_text("text/plain", b"  one\n\ntwo\t three  "), "one two three");
        assert_eq!(normalize_text("text/plain", &[b'a', 0xff, b'b']), "a\u{fffd}b");
        // Another media type is read as plain text.
        assert_eq!(normalize_text("application/octet-stream", b"x  y"), "x y");
    }

    #[test]
    fn a_tag_with_attributes_and_an_unclosed_script_do_not_leak() {
        let text = normalize_text("text/html", b"<a href=\"x>y\" class='z'>link</a> tail <script>never closed");
        assert!(text.starts_with("link tail"), "{text}");
        assert!(!text.contains("never"), "{text}");
    }

    #[test]
    fn a_passage_is_found_across_a_line_break_and_not_with_a_word_changed() {
        let text = normalize_text("text/plain", b"Student records are kept for\n  seven years after the student leaves.");
        assert!(passage_in(&text, "records are kept for seven years after the student"));
        assert!(passage_in(&text, "records are kept for\nseven   years after the student"));
        assert!(!passage_in(&text, "records are kept for eight years after the student"));
    }

    #[test]
    fn a_passage_must_be_between_20_and_2000_characters() {
        let text = "x".repeat(5000);
        assert!(!passage_in(&text, &"x".repeat(19)));
        assert!(passage_in(&text, &"x".repeat(20)));
        assert!(passage_in(&text, &"x".repeat(2000)));
        assert!(!passage_in(&text, &"x".repeat(2001)));
        // The length is of the passage as written, in characters.
        assert!(!passage_in("ééééééééééééééééééé", "ééééééééééééééééééé"), "19 characters");
        assert!(passage_in("éééééééééééééééééééé", "éééééééééééééééééééé"), "20 characters");
    }

    #[test]
    fn a_binding_is_enforced_only_when_its_target_is_there_and_active() {
        let lookup = |kind: &str, target: &str| kind == "cedar_policy" && target == "p1";
        assert_eq!(binding_state("cedar_policy", "p1", &lookup), BindingState::Enforced);
        assert_eq!(binding_state("cedar_policy", "gone", &lookup), BindingState::Missing);
        assert_eq!(binding_state("retention_policy", "p1", &lookup), BindingState::Missing);
    }
}
