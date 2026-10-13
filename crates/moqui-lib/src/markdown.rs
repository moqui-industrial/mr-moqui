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

//! CommonMark to HTML, the `.md` and `.markdown` template renderers.
//!
//! Java renders these with flexmark, the tables and TOC extensions, and raw HTML passed through. This
//! is CommonMark plus GitHub tables, and raw HTML is passed through as Java does: a markdown file is
//! trusted like any other template file. The `[TOC]` block of the flexmark TOC extension is not
//! rendered; it stays a paragraph.

use pulldown_cmark::{html, Options, Parser};

/// Render `source` as HTML: CommonMark with tables. Raw HTML blocks and inline HTML are written as is.
pub fn to_html(source: &str) -> String {
    let parser = Parser::new_ext(source, Options::ENABLE_TABLES);
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

    #[test]
    fn tables_and_raw_html_pass_through() {
        let html =
            to_html("| a | b |\n|---|---|\n| 1 | 2 |\n\n<div class=\"note\">kept</div>\n\ntext <b>bold</b> & more\n");
        assert!(html.contains("<table>") && html.contains("<td>1</td>"), "{html}");
        assert!(html.contains("<div class=\"note\">kept</div>"), "{html}");
        assert!(html.contains("text <b>bold</b> &amp; more"), "{html}");
    }
}
