/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! CommonMark to HTML. Screen and wiki widgets call this later; nothing in the screen model does yet.

use pulldown_cmark::{html, Parser};

/// Render `source` as HTML. The parser is the crate default: no tables, no footnotes, no raw HTML pass-through beyond CommonMark.
pub fn to_html(source: &str) -> String {
    let parser = Parser::new(source);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

#[cfg(test)]
mod tests {
    use super::to_html;

    #[test]
    fn heading_and_list_become_html() {
        let html = to_html("# Title\n\n- one\n- two\n");
        assert!(html.contains("<h1>Title</h1>"), "{html}");
        assert!(html.contains("<li>one</li>"), "{html}");
        assert!(html.contains("<li>two</li>"), "{html}");
    }
}
