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

//! Saved finds and the user's columns of a form-list through the implicit `formSaveFind` and `formSelectColumns`
//! transitions, and the `screen.user.page.size` preference.

use super::{basic, request, World};
use crate::screen::serve_at;
use crate::web::WebRequest;

const SCREEN: &str = r#"<screen require-authentication="false">
    <actions>
        <script>rows = [{a: 'A1', b: 'B1', c: 'C1'}]</script>
    </actions>
    <widgets>
        <label text="a=${a || ''} size=${pageSize || ''}"/>
        <form-list name="Rows" list="rows" select-columns="true" saved-finds="true">
            <field name="a"><header-field title="Col A"><text-find/></header-field><default-field><display/></default-field></field>
            <field name="b"><header-field title="Col B"/><default-field><display/></default-field></field>
            <field name="c"><header-field title="Col C"/><default-field><display/></default-field></field>
        </form-list>
    </widgets>
</screen>"#;

struct Site {
    dir: std::path::PathBuf,
    root: String,
    world: World,
}

impl Site {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("moqui-saved-finds-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Find.xml"), SCREEN).unwrap();
        let world = World::open("");
        world.user("ada", None);
        let root = dir.join("Find.xml").to_string_lossy().into_owned();
        Self { dir, root, world }
    }

    fn send(&self, mut request: WebRequest) -> (u16, String, String) {
        request.set_header("authorization", basic("ada"));
        let response = serve_at(&self.world.factory, request, &self.root);
        let location = response.header("location").unwrap_or_default().to_string();
        (response.status, location, response.body_text())
    }

    fn get(&self, query: &[(&str, &str)]) -> String {
        let mut get = request("GET", "/");
        get.query = query
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        let (status, _, body) = self.send(get);
        assert_eq!(status, 200, "{body}");
        body
    }

    fn post(&self, path: &str, body: &str) -> (u16, String) {
        let mut post = request("POST", path);
        post.set_header("content-type", "application/x-www-form-urlencoded");
        post.body = body.as_bytes().to_vec();
        let (status, location, text) = self.send(post);
        assert_eq!(status, 302, "{text}");
        (status, location)
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The header titles of the form-list in the order the page shows them.
fn titles(html: &str) -> Vec<&'static str> {
    let html = &html[html.find("<table").unwrap_or(0)..];
    let mut found: Vec<(usize, &'static str)> = ["Col A", "Col B", "Col C"]
        .into_iter()
        .filter_map(|title| html.find(&format!(">{title}<")).map(|at| (at, title)))
        .collect();
    found.sort();
    found.into_iter().map(|(_, title)| title).collect()
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[test]
fn the_column_dialog_saves_the_users_columns_and_reset_restores_the_form() {
    let site = Site::new("columns");
    assert_eq!(titles(&site.get(&[])), vec!["Col A", "Col B", "Col C"]);
    let tree = r#"[{"id":"hidden","order":0,"children":[{"id":"b","order":0}]},
        {"id":"column_0","order":1,"children":[{"id":"c","order":0}]},{"id":"column_1","order":2,"children":[{"id":"a","order":0}]}]"#;
    let (_, location) = site.post(
        "/formSelectColumns",
        &format!("formLocation=Rows&SaveColumns=Save&columnsTree={}", encode(tree)),
    );
    assert!(!location.contains("columnsTree"), "{location}");
    let html = site.get(&[]);
    assert_eq!(titles(&html), vec!["Col C", "Col A"], "{html}");
    assert!(html.contains(">C1<") && !html.contains(">B1<"), "{html}");
    site.post("/formSelectColumns", "formLocation=Rows&ResetColumns=Reset");
    assert_eq!(titles(&site.get(&[])), vec!["Col A", "Col B", "Col C"]);
}

#[test]
fn a_saved_find_keeps_its_parameters_and_can_be_the_default_then_deleted() {
    let site = Site::new("finds");
    let (_, location) = site.post("/formSaveFind", "formLocation=Rows&a=foo&_findDescription=Mine");
    let id = location
        .split("formListFindId=")
        .nth(1)
        .map(|rest| rest.split('&').next().unwrap_or("").to_string())
        .unwrap_or_else(|| panic!("no saved find in {location}"));
    assert!(!id.is_empty());
    // The find dialog lists it, in html and in qvt (whose template the Vue compiler takes).
    let html = site.get(&[("a", "foo")]);
    assert!(
        html.contains("<ul class=\"saved-finds\">") && html.contains(">Mine</a>"),
        "{html}"
    );
    assert!(
        html.contains("name=\"DeleteFind\"") && html.contains("name=\"MakeDefault\""),
        "{html}"
    );
    let qvt = site.get(&[("renderMode", "qvt"), ("a", "foo")]);
    assert!(qvt.contains("Mine") && qvt.contains("name=\"DeleteFind\""), "{qvt}");
    super::qvt::assert_compiles(&["saved finds".to_string()], &[qvt]);
    // The saved find's parameters apply when the request has none of its own; a parameter of the request wins.
    assert!(site.get(&[("formListFindId", &id)]).contains("a=foo"));
    assert!(site.get(&[("formListFindId", &id), ("a", "bar")]).contains("a=bar"));
    assert!(!site.get(&[]).contains("a=foo"));
    // Made the default for the screen, it applies with no formListFindId; _clear turns it off for a request.
    site.post(
        "/formSaveFind",
        &format!("formLocation=Rows&formListFindId={id}&MakeDefault=Y"),
    );
    assert!(site.get(&[]).contains("a=foo"));
    assert!(!site.get(&[("formListFindId", "_clear")]).contains("a=foo"));
    site.post(
        "/formSaveFind",
        &format!("formLocation=Rows&formListFindId={id}&DeleteFind=Y"),
    );
    assert!(!site.get(&[]).contains("a=foo"));
    assert!(!site.get(&[("formListFindId", &id)]).contains("a=foo"));
}

#[test]
fn a_page_size_is_kept_as_the_users_preference() {
    let site = Site::new("page-size");
    assert!(site.get(&[]).contains("size=<"), "no size yet");
    site.get(&[("pageSize", "7")]);
    assert!(site.get(&[]).contains("size=7"));
}
