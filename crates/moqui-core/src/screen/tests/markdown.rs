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

//! Markdown text widgets: a `.md` location and a `.md.fancy` template in html and qvt, and the
//! markdown source in text mode. This is how a wiki page is shown: a `render-mode` text whose
//! location is the page file.

use crate::context::ExecutionContextFactory;
use crate::screen::make_render;

fn render(mode: &str) -> String {
    let context = ExecutionContextFactory::test_context();
    let root = format!(
        "{}/src/screen/testdata/Markdown/Markdown.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = make_render(context.factory())
        .root_screen(root)
        .screen_path("")
        .render_mode(mode)
        .render()
        .unwrap_or_else(|err| panic!("{mode}: {err}"));
    assert_eq!(output.status, 200, "{mode}");
    output.body_text()
}

fn assert_markdown_html(out: &str) {
    assert!(out.contains("<h1>Page Title</h1>"), "{out}");
    assert!(
        out.contains("<p>Some <em>emphasis</em> and a <a href=\"https://moqui.org/\">link</a>.</p>"),
        "{out}"
    );
    assert!(out.contains("<table>") && out.contains("<td>one</td>"), "{out}");
    assert!(out.contains("<div class=\"note\">raw html kept</div>"), "{out}");
    // the .md.fancy template: markdown first, then the template with HTML escaped output
    assert!(out.contains("<h2>Hello Ada &amp; co</h2>"), "{out}");
    assert!(out.contains("<li>greeted <strong>Ada &amp; co</strong></li>"), "{out}");
    assert!(!out.contains("<#") && !out.contains("${"), "{out}");
}

#[test]
fn a_markdown_text_renders_as_html() {
    assert_markdown_html(&render("html"));
}

#[test]
fn a_markdown_text_renders_in_qvt_and_the_template_compiles() {
    let out = render("qvt");
    assert_markdown_html(&out);
    super::qvt::assert_compiles(&["markdown".to_string()], std::slice::from_ref(&out));
}

#[test]
fn text_mode_shows_the_markdown_source() {
    let out = render("text");
    assert!(out.contains("# Page Title"), "{out}");
    assert!(out.contains("Some *emphasis*"), "{out}");
    assert!(!out.contains("<h1>") && !out.contains("Hello"), "{out}");
}

/// The `sri` and `ec.resource` calls the wiki screens make before they render a page: the path name lists of
/// `getScreenUrlInfo()`, `getCurrentScreenUrl().getScreenOnlyPath()` (no extra path), `baseLinkUrl`, and the template
/// extensions for the page type drop-down.
#[test]
fn the_wiki_screens_url_and_template_calls_answer() {
    let dir = std::env::temp_dir().join(format!("moqui-wiki-sri-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Root")).unwrap();
    std::fs::write(
        dir.join("Root.xml"),
        r#"<screen require-authentication="false"><subscreens default-item="Page"/><widgets><subscreens-active/></widgets></screen>"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("Root/Page.xml"),
        r#"<screen require-authentication="false" allow-extra-path="true">
            <actions><script><![CDATA[
                const info = sri.getScreenUrlInfo()
                fullPath = info.fullPathNameList.join('/')
                extraPath = info.extraPathNameList.join('/')
                screenOnly = sri.getCurrentScreenUrl().getScreenOnlyPath()
                sri.baseLinkUrl('https://example.com/wiki')
                baseLink = sri.getBaseLinkUrl()
                sri.baseLinkUrl(null)
                baseLinkAfter = String(sri.getBaseLinkUrl())
                extensions = ec.resource.getTemplateRendererExtensionSet().includes('md')
            ]]></script></actions>
            <widgets><label text="full=${fullPath} extra=${extraPath} only=${screenOnly} base=${baseLink} after=${baseLinkAfter} md=${extensions}"/></widgets>
        </screen>"#,
    )
    .unwrap();
    let context = ExecutionContextFactory::test_context();
    let output = make_render(context.factory())
        .root_screen(dir.join("Root.xml").to_string_lossy().into_owned())
        .screen_path("Page/SPACE/Some Page")
        .render_mode("text")
        .render()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let out = output.body_text();
    assert!(
        out.contains("full=Page/SPACE/Some Page extra=SPACE/Some Page only=/Page "),
        "{out}"
    );
    assert!(
        out.contains("base=https://example.com/wiki after=null md=true"),
        "{out}"
    );
}
