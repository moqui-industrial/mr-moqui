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

//! A form posted to a transition through the serve path: the session token, the redirect, the messages and
//! typed values that the next html page shows once, a multipart upload, and the error pages.

use std::path::PathBuf;

use super::{request, World};
use crate::screen::serve_at;
use crate::web::{WebRequest, WebResponse};

struct Site {
    dir: PathBuf,
    root: String,
}

impl Site {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("moqui-forms-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let messages = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../runtime/base-component/webroot/screen/includes/Messages.html.fancy");
        let screen = r#"<screen require-authentication="false">
    <transition name="save">
        <actions><script><![CDATA[
            if (!title || title == 'bad') ec.message.addError('Title is not valid')
            else ec.message.addMessage('Saved ' + title)
        ]]></script></actions>
        <default-response url="."/>
        <error-response url="."/>
    </transition>
    <transition name="upload">
        <actions><script><![CDATA[
            const doc = ec.web.multiPartParameters.doc
            ec.web.sendJsonResponse({title: title, name: doc.name, type: doc.contentType, size: doc.size,
                text: doc.getString(), count: ec.web.multiPartParameters._fileUploadList.length})
        ]]></script></actions>
        <default-response type="none"/>
    </transition>
    <transition name="check">
        <actions><script><![CDATA[
            ec.message.addMessage('Check this', 'warning')
            ec.message.addValidationError('Edit', 'title', 'save#Thing', 'Title needs a letter')
        ]]></script></actions>
        <default-response url="."/>
        <error-response url="."/>
    </transition>
    <transition name="ajax" require-session-token="false">
        <actions><script>ec.web.sendJsonResponse({ok: true})</script></actions>
        <default-response type="none"/>
    </transition>
    <widgets>
        <render-mode><text type="html" location="MESSAGES"/></render-mode>
        <form-single name="Edit" transition="save">
            <field name="title"><default-field><text-line/></default-field></field>
            <field name="go"><default-field><submit/></default-field></field>
        </form-single>
    </widgets>
</screen>"#
            .replace("MESSAGES", &messages.display().to_string());
        std::fs::write(dir.join("Form.xml"), screen).unwrap();
        let root = dir.join("Form.xml").to_string_lossy().into_owned();
        Self { dir, root }
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The cookies a response sets, as a `Cookie` header value.
fn cookies(response: &WebResponse, jar: &mut Vec<(String, String)>) {
    for (name, value) in &response.headers {
        if name.eq_ignore_ascii_case("set-cookie") {
            let pair = value.split(';').next().unwrap_or("");
            if let Some((key, val)) = pair.split_once('=') {
                jar.retain(|(existing, _)| existing != key);
                jar.push((key.to_string(), val.to_string()));
            }
        }
    }
}

fn with_cookies(mut request: WebRequest, jar: &[(String, String)]) -> WebRequest {
    if !jar.is_empty() {
        let header: Vec<String> = jar.iter().map(|(key, val)| format!("{key}={val}")).collect();
        request.set_header("cookie", header.join("; "));
    }
    request
}

fn token_of(html: &str) -> String {
    let marker = "name=\"moquiSessionToken\" value=\"";
    let start = html.find(marker).unwrap_or_else(|| panic!("no token in {html}")) + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_string()
}

fn form_post(path: &str, body: &str) -> WebRequest {
    let mut post = request("POST", path);
    post.set_header("content-type", "application/x-www-form-urlencoded");
    post.body = body.as_bytes().to_vec();
    post
}

#[test]
fn a_validation_error_shows_by_its_field_and_a_message_keeps_its_type_after_the_redirect() {
    let site = Site::new("validation");
    let world = World::open("");
    let mut jar = Vec::new();
    let page = serve_at(&world.factory, request("GET", "/"), &site.root);
    cookies(&page, &mut jar);
    let token = token_of(&page.body_text());
    let posted = serve_at(
        &world.factory,
        with_cookies(
            form_post(
                "/check",
                &format!("title=x&moquiFormName=Edit&moquiSessionToken={token}"),
            ),
            &jar,
        ),
        &site.root,
    );
    assert_eq!(posted.status, 302, "{}", posted.body_text());
    cookies(&posted, &mut jar);
    let html = serve_at(&world.factory, with_cookies(request("GET", "/"), &jar), &site.root).body_text();
    assert!(html.contains("data-type=\"warning\""), "{html}");
    assert!(html.contains("Check this"), "{html}");
    // The field's own error is under the field, not a line at the top of the page.
    assert!(
        html.contains("<div class=\"field-error\" role=\"alert\">Title needs a letter</div>"),
        "{html}"
    );
    assert!(!html.contains("data-type=\"danger\""), "{html}");
}

#[test]
fn a_saved_validation_error_reaches_the_qvt_m_form_of_its_form() {
    let site = Site::new("validation-qvt");
    let world = World::open("");
    let mut jar = Vec::new();
    let page = serve_at(&world.factory, request("GET", "/"), &site.root);
    cookies(&page, &mut jar);
    let token = token_of(&page.body_text());
    let posted = serve_at(
        &world.factory,
        with_cookies(
            form_post(
                "/check",
                &format!("title=x&moquiFormName=Edit&moquiSessionToken={token}"),
            ),
            &jar,
        ),
        &site.root,
    );
    assert_eq!(posted.status, 302, "{}", posted.body_text());
    cookies(&posted, &mut jar);
    let mut get = request("GET", "/");
    get.query.push(("renderMode".into(), "qvt".into()));
    let qvt = serve_at(&world.factory, with_cookies(get, &jar), &site.root).body_text();
    assert!(qvt.contains(":validation-errors-initial="), "{qvt}");
    assert!(qvt.contains("Title needs a letter"), "{qvt}");
    assert!(qvt.contains("fieldPretty"), "{qvt}");
}

#[test]
fn a_posted_form_redirects_and_the_next_page_shows_its_messages_and_values_once() {
    let site = Site::new("flash");
    let world = World::open("");
    let mut jar = Vec::new();

    let page = serve_at(&world.factory, request("GET", "/"), &site.root);
    assert_eq!(page.status, 200, "{}", page.body_text());
    cookies(&page, &mut jar);
    let html = page.body_text();
    assert!(
        html.contains("<form name=\"Edit\" id=\"Edit\" method=\"post\" action=\"/save\">"),
        "{html}"
    );
    let token = token_of(&html);
    assert!(!html.contains("class=\"alert\""), "{html}");

    // without the token the transition is refused
    let refused = serve_at(
        &world.factory,
        with_cookies(form_post("/save", "title=bad&moquiFormName=Edit"), &jar),
        &site.root,
    );
    assert_eq!(refused.status, 401, "{}", refused.body_text());

    // a bad value: redirect back to the form
    let bad = serve_at(
        &world.factory,
        with_cookies(
            form_post(
                "/save",
                &format!("title=bad&moquiFormName=Edit&moquiSessionToken={token}"),
            ),
            &jar,
        ),
        &site.root,
    );
    assert_eq!(bad.status, 302, "{}", bad.body_text());
    assert_eq!(bad.header("Location"), Some("/"));
    cookies(&bad, &mut jar);

    let again = serve_at(&world.factory, with_cookies(request("GET", "/"), &jar), &site.root);
    let html = again.body_text();
    assert!(
        html.contains("<div class=\"alert\" data-type=\"danger\" role=\"alert\">"),
        "{html}"
    );
    assert!(html.contains("Title is not valid</div>"), "{html}");
    // the typed value is back in the field
    assert!(html.contains("name=\"title\" value=\"bad\""), "{html}");

    // shown once
    let third = serve_at(&world.factory, with_cookies(request("GET", "/"), &jar), &site.root);
    let html = third.body_text();
    assert!(!html.contains("Title is not valid"), "{html}");
    assert!(html.contains("name=\"title\" value=\"\""), "{html}");

    // a good value: the message is info and the value is not carried back
    let good = serve_at(
        &world.factory,
        with_cookies(
            form_post(
                "/save",
                &format!("title=Fine&moquiFormName=Edit&moquiSessionToken={token}"),
            ),
            &jar,
        ),
        &site.root,
    );
    assert_eq!(good.status, 302, "{}", good.body_text());
    let after = serve_at(&world.factory, with_cookies(request("GET", "/"), &jar), &site.root).body_text();
    assert!(after.contains("<div class=\"alert\" data-type=\"info\""), "{after}");
    assert!(after.contains("Saved Fine</div>"), "{after}");
    assert!(after.contains("name=\"title\" value=\"\""), "{after}");

    // the token also comes as a header, as the client sends it
    let mut ajax = form_post("/save", "title=Header");
    ajax.set_header("x-csrf-token", token);
    let by_header = serve_at(&world.factory, with_cookies(ajax, &jar), &site.root);
    assert_eq!(by_header.status, 302, "{}", by_header.body_text());

    // a transition that turns the check off
    let open = serve_at(&world.factory, form_post("/ajax", "a=1"), &site.root);
    assert_eq!(open.status, 200, "{}", open.body_text());
}

/// A multipart field: its name, its value and for a file the file name and type.
type Part<'a> = (&'a str, &'a str, Option<(&'a str, &'a str)>);

fn multipart(parts: &[Part<'_>]) -> (String, Vec<u8>) {
    let mut body = Vec::new();
    for (name, value, file) in parts {
        body.extend_from_slice(b"--BoUnD\r\n");
        match file {
            Some((file_name, content_type)) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes()),
        }
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--BoUnD--\r\n");
    ("multipart/form-data; boundary=BoUnD".to_string(), body)
}

fn upload_request(token: &str, parts: &[Part<'_>]) -> WebRequest {
    let mut all = vec![("moquiSessionToken", token, None)];
    all.extend_from_slice(parts);
    let (content_type, body) = multipart(&all);
    let mut post = request("POST", "/upload");
    post.set_header("content-type", content_type);
    post.body = body;
    post
}

#[test]
fn a_multipart_form_gives_parameters_and_the_uploaded_file() {
    let site = Site::new("upload");
    let world = World::open("");
    let mut jar = Vec::new();
    let page = serve_at(&world.factory, request("GET", "/"), &site.root);
    cookies(&page, &mut jar);
    let token = token_of(&page.body_text());

    let parts = [
        ("title", "Report", None),
        ("doc", "hello,world\n", Some(("C:\\fakepath\\data.csv", "text/csv"))),
    ];
    let response = serve_at(
        &world.factory,
        with_cookies(upload_request(&token, &parts), &jar),
        &site.root,
    );
    assert_eq!(response.status, 200, "{}", response.body_text());
    let json = crate::service::parse_json(&response.body_text()).unwrap();
    let get = |key: &str| match &json {
        moqui_lib::object::Value::Map(map) => moqui_lib::object::to_plain_string(map.get(key).unwrap()),
        other => panic!("{other:?}"),
    };
    assert_eq!(get("title"), "Report");
    assert_eq!(get("name"), "data.csv");
    assert_eq!(get("type"), "text/csv");
    assert_eq!(get("size"), "12");
    assert_eq!(get("text"), "hello,world\n");
    assert_eq!(get("count"), "1");

    // without the token a multipart post is refused as a urlencoded one is
    let (content_type, body) = multipart(&parts);
    let mut bare = request("POST", "/upload");
    bare.set_header("content-type", content_type);
    bare.body = body;
    assert_eq!(
        serve_at(&world.factory, with_cookies(bare, &jar), &site.root).status,
        401
    );

    // a program is not accepted unless the web app allows it
    let program = [("doc", "MZ\u{90}\u{0}", Some(("setup.exe", "application/octet-stream")))];
    let refused = serve_at(
        &world.factory,
        with_cookies(upload_request(&token, &program), &jar),
        &site.root,
    );
    assert_eq!(refused.status, 415, "{}", refused.body_text());
    assert!(refused.body_text().contains("setup.exe"), "{}", refused.body_text());
    let script = [("doc", "#!/bin/sh\necho hi", Some(("run.sh", "text/x-sh")))];
    assert_eq!(
        serve_at(
            &world.factory,
            with_cookies(upload_request(&token, &script), &jar),
            &site.root
        )
        .status,
        415
    );

    // a body that is not multipart at all
    let mut broken = request("POST", "/upload");
    broken.set_header("content-type", "multipart/form-data; boundary=BoUnD");
    broken.body = b"--BoUnD\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nno end".to_vec();
    let response = serve_at(&world.factory, with_cookies(broken, &jar), &site.root);
    assert_eq!(response.status, 400, "{}", response.body_text());
}

#[test]
fn the_upload_cap_is_webapp_max_upload_and_other_bodies_stop_at_webapp_max_body() {
    let site = Site::new("cap");
    let world = World::open("");
    moqui_lib::env::set_property("webapp_max_upload", "2k");
    let big = "x".repeat(3000);
    let parts = [("doc", big.as_str(), Some(("a.txt", "text/plain")))];
    let (content_type, body) = multipart(&parts);
    let mut post = request("POST", "/upload");
    post.set_header("content-type", content_type.clone());
    post.body = body;
    let response = serve_at(&world.factory, post, &site.root);
    moqui_lib::env::clear_property("webapp_max_upload");
    assert_eq!(response.status, 413, "{}", response.body_text());

    // a form post has its own cap, webapp_max_body (default 10 MiB), even when uploads may be larger
    moqui_lib::env::set_property("webapp_max_body", "1m");
    let mut large = request("POST", "/save");
    large.set_header("content-type", "application/x-www-form-urlencoded");
    large.body = vec![b'a'; 1024 * 1024 + 1];
    let status = serve_at(&world.factory, large, &site.root).status;
    moqui_lib::env::clear_property("webapp_max_body");
    assert_eq!(status, 413);
}

#[test]
fn a_failed_request_in_a_browser_is_an_html_error_page_and_elsewhere_json() {
    let site = Site::new("errors");
    let world = World::open("");
    let mut get = request("GET", "/NoSuchScreen");
    get.set_header("accept", "text/html,application/xhtml+xml");
    let page = serve_at(&world.factory, get, &site.root);
    // the fixture root has no error screens, so the host falls back to the JSON body
    assert_eq!(page.status, 404, "{}", page.body_text());
    let json = serve_at(&world.factory, request("GET", "/NoSuchScreen"), &site.root);
    assert_eq!(json.status, 404);
    assert_eq!(json.header("Content-Type"), Some("application/json"));
}

/// A root screen that wraps what is below it in the real header and footer, and mounts the real error screens.
fn shell_root(name: &str) -> Site {
    let dir = std::env::temp_dir().join(format!("moqui-shell-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let webroot = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/webroot/screen");
    let webroot = webroot.canonicalize().unwrap();
    let screen = r#"<screen require-authentication="false">
    <always-actions>
        <set field="html_scripts" from="new Set()"/>
        <set field="footer_scripts" from="new Set()"/>
        <set field="html_stylesheets" from="new Set()"/>
        <set field="footer_stylesheets" from="new Set()"/>
    </always-actions>
    <subscreens default-item="Home">
        <subscreens-item name="error" location="WEBROOT/webroot/error.xml"/>
    </subscreens>
    <widgets>
        <render-mode><text type="html" location="WEBROOT/includes/Header.html.fancy" no-boundary-comment="true"/></render-mode>
        <render-mode><text template="true">${sri.renderSubscreen()}</text></render-mode>
        <render-mode><text type="html" location="WEBROOT/includes/Footer.html.fancy"/></render-mode>
    </widgets>
</screen>"#
        .replace("WEBROOT", &webroot.display().to_string());
    std::fs::write(dir.join("Shell.xml"), screen).unwrap();
    std::fs::create_dir_all(dir.join("Shell")).unwrap();
    std::fs::write(
        dir.join("Shell/Home.xml"),
        r#"<screen require-authentication="false" default-menu-title="Home Page"><widgets><label text="the home page"/></widgets></screen>"#,
    )
    .unwrap();
    let root = dir.join("Shell.xml").to_string_lossy().into_owned();
    Site { dir, root }
}

#[test]
fn the_shell_wraps_a_page_and_the_real_error_pages_show_the_error_context() {
    let site = shell_root("errors");
    let world = World::open("");
    let mut home = request("GET", "/");
    home.set_header("accept", "text/html");
    let page = serve_at(&world.factory, home, &site.root);
    let html = page.body_text();
    assert_eq!(page.status, 200, "{html}");
    assert!(html.starts_with("<!DOCTYPE html>\n<html lang="), "{html}");
    assert!(html.contains("<meta name=\"moquiSessionToken\" content=\""), "{html}");
    assert!(html.contains("<title>Shell - Home Page</title>"), "{html}");
    assert!(html.contains("<span>the home page</span>"), "{html}");
    assert!(html.trim_end().ends_with("</body>\n</html>"), "{html}");

    let mut missing = request("GET", "/NoSuchScreen");
    missing.set_header("accept", "text/html");
    let page = serve_at(&world.factory, missing, &site.root);
    let html = page.body_text();
    assert_eq!(page.status, 404, "{html}");
    assert!(
        html.contains("<h1><i class=\"fa fa-link text-danger\"></i> Page Not Found (404)</h1>"),
        "{html}"
    );
    assert!(html.contains("Could not find <code>NoSuchScreen</code>"), "{html}");
    assert!(
        html.contains("<link rel=\"stylesheet\" href=\"/css/moqui-html.css"),
        "{html}"
    );
    assert!(html.contains("<script src=\"/js/moqui-html.js"), "{html}");
    assert!(html.contains("defer"), "{html}");
    let lower = html.to_ascii_lowercase();
    assert!(!lower.contains("jquery") && !lower.contains("bootstrap"), "{html}");

    // an API path keeps the JSON error
    let mut rest = request("GET", "/rest/NoSuchService");
    rest.set_header("accept", "text/html");
    assert_eq!(
        serve_at(&world.factory, rest, &site.root).header("Content-Type"),
        Some("application/json")
    );
}
