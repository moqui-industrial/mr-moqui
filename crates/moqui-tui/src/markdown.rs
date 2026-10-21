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

//! Markdown as the blocks of an `html` node (`markdown-view`, and the F4 preview of an `editor-type="md"` text area):
//! headings, paragraphs, list items with their markers, quotes, code and rules, with each link's text followed by `[n]`
//! and its url the n-th entry of the links, as the server's HTML conversion writes them. Emphasis is dropped; the text
//! stays.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::model::{clean_text, HtmlBlock, HtmlLink};

/// The blocks and links of `source`.
pub fn blocks(source: &str) -> (Vec<HtmlBlock>, Vec<HtmlLink>) {
    let mut out = Out::default();
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(source, options) {
        out.event(event);
    }
    out.close();
    (out.blocks, out.links)
}

#[derive(Default)]
struct Out {
    blocks: Vec<HtmlBlock>,
    links: Vec<HtmlLink>,
    /// The block being filled: its kind and text, open when `kind` is not empty.
    kind: String,
    text: String,
    marker: String,
    depth: u8,
    quotes: u8,
    /// Each open list: the next number of an ordered one, None for bullets.
    lists: Vec<Option<u64>>,
    /// The link being read: its url, and where its text starts.
    link: Option<(String, usize)>,
    in_code: bool,
    cells: Vec<String>,
}

impl Out {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push(&text),
            Event::Code(text) => self.push(&text),
            Event::SoftBreak => self.push(" "),
            Event::HardBreak => self.push("\n"),
            Event::Rule => {
                self.close();
                self.blocks.push(block("rule", String::new(), 0, String::new()));
            }
            Event::TaskListMarker(done) => self.push(if done { "[x] " } else { "[ ] " }),
            Event::Html(_) | Event::InlineHtml(_) | Event::FootnoteReference(_) => {}
            Event::InlineMath(text) | Event::DisplayMath(text) => self.push(&text),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                // A list item's first paragraph is the item.
                if self.kind != "li" || !self.text.trim().is_empty() {
                    self.open(if self.quotes > 0 { "quote" } else { "p" }, String::new());
                }
            }
            Tag::Heading { level, .. } => {
                let kind = match level {
                    HeadingLevel::H1 => "h1",
                    HeadingLevel::H2 => "h2",
                    HeadingLevel::H3 => "h3",
                    HeadingLevel::H4 => "h4",
                    HeadingLevel::H5 => "h5",
                    HeadingLevel::H6 => "h6",
                };
                self.open(kind, String::new());
            }
            Tag::BlockQuote(_) => {
                self.close();
                self.quotes += 1;
            }
            Tag::CodeBlock(_) => {
                self.open("pre", String::new());
                self.in_code = true;
            }
            Tag::List(start) => {
                self.close();
                self.lists.push(start);
            }
            Tag::Item => {
                let marker = match self.lists.last_mut() {
                    Some(Some(next)) => {
                        let marker = format!("{next}.");
                        *next += 1;
                        marker
                    }
                    _ => "•".to_string(),
                };
                self.open("li", marker);
            }
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.link = Some((dest_url.to_string(), self.text.len()));
            }
            Tag::TableRow | Tag::TableHead => {
                self.close();
                self.cells.clear();
            }
            Tag::TableCell => self.text.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item => self.close(),
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.close();
            }
            TagEnd::BlockQuote(_) => {
                self.close();
                self.quotes = self.quotes.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.close();
                self.lists.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                if let Some((url, start)) = self.link.take() {
                    let text = self.text.get(start..).unwrap_or("").trim().to_string();
                    self.links.push(HtmlLink {
                        text: clean_text(&text),
                        url: clean_text(&url),
                    });
                    self.text.push_str(&format!("[{}]", self.links.len()));
                }
            }
            TagEnd::TableCell => self.cells.push(std::mem::take(&mut self.text).trim().to_string()),
            TagEnd::TableRow | TagEnd::TableHead => {
                let row = std::mem::take(&mut self.cells).join(" │ ");
                self.blocks.push(block("row", clean_text(&row), 0, String::new()));
            }
            _ => {}
        }
    }

    fn push(&mut self, text: &str) {
        if self.kind.is_empty() && self.cells.is_empty() && self.link.is_none() && !text.trim().is_empty() {
            self.open(if self.quotes > 0 { "quote" } else { "p" }, String::new());
        }
        self.text.push_str(text);
    }

    fn open(&mut self, kind: &str, marker: String) {
        self.close();
        self.kind = kind.to_string();
        self.marker = marker;
        self.depth = u8::try_from(self.lists.len()).unwrap_or(u8::MAX).max(self.quotes);
    }

    fn close(&mut self) {
        if self.kind.is_empty() {
            return;
        }
        let kind = std::mem::take(&mut self.kind);
        let text = std::mem::take(&mut self.text);
        let text = if kind == "pre" {
            text.trim_end_matches('\n').to_string()
        } else {
            text.trim().to_string()
        };
        if !text.is_empty() || kind == "li" {
            self.blocks.push(block(
                &kind,
                clean_text(&text),
                self.depth,
                std::mem::take(&mut self.marker),
            ));
        }
        self.marker.clear();
    }
}

fn block(kind: &str, text: String, depth: u8, marker: String) -> HtmlBlock {
    HtmlBlock {
        kind: kind.to_string(),
        text,
        depth,
        marker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_lists_and_links_read_as_the_html_conversion_writes_them() {
        let (blocks, links) = blocks("# Title\n\nSome *text* with [a link](https://x.example).\n\n- one\n- two\n\n1. first\n\n> quoted\n\n```\ncode\n```\n\n---\n");
        let kinds: Vec<(&str, &str, &str)> = blocks
            .iter()
            .map(|b| (b.kind.as_str(), b.text.as_str(), b.marker.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("h1", "Title", ""),
                ("p", "Some text with a link[1].", ""),
                ("li", "one", "•"),
                ("li", "two", "•"),
                ("li", "first", "1."),
                ("quote", "quoted", ""),
                ("pre", "code", ""),
                ("rule", "", ""),
            ]
        );
        assert_eq!(links[0].url, "https://x.example");
        assert_eq!(links[0].text, "a link");
    }
}
