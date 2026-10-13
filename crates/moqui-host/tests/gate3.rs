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

//! Gate 3, started: a real framework screen rendered in the text, csv, xml and html-print modes through the HTTP
//! server with the seeded embedded-Postgres data. The later slices add the other modes to this file.
//!
//! The fixture is `component://tools/screen/System/Security/ArtifactGroup/ArtifactGroupDetail.xml`.
//! It has what the gate asks for:
//!
//! - a `form-single` (`ArtifactGroupForm`, a `display` and a `text-line` over the `artifactGroup` value),
//! - a `form-list` with row-actions (`ArtifactAuthzList`, with `display-entity`, `drop-down` with
//!   `entity-options`, `link` and a `section-iterate` inside a field) and three more form-lists,
//! - sections (the `section-iterate` of entity filter sets), and containers (`container-box`),
//! - actions that are only `entity-find` and `entity-find-one`: no service, no mail, no search, no LLM.
//!
//! The other candidates either keep their forms in dialogs, which the text modes do not show, or
//! call services, search or the file system.
//!
//! The tools screens are not mounted under any app by the conf in this repository, so the test adds the
//! mount in a `conf/MoquiDevConf.xml` of its temporary runtime, which is where a deployment would.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moqui_host::{load_data, start_server, LoadFlags};

const CONF: &str = r#"<moqui-conf>
    <screen-facade>
        <screen location="component://webroot/screen/webroot.xml">
            <subscreens-item name="System" menu-title="System" location="component://tools/screen/System.xml"/>
            <subscreens-item name="Tools" menu-title="Tools" location="component://tools/screen/Tools.xml"/>
        </screen>
        <screen location="component://webroot/screen/webroot/apps.xml">
            <subscreens-item name="System" menu-title="System" location="component://tools/screen/System.xml"/>
        </screen>
    </screen-facade>
</moqui-conf>"#;

/// One embedded Postgres at a time: the tests set process-wide properties while they start.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The seeded server the gate tests share the setup of. Fields drop in order: the server stops before its directories go.
struct Gate {
    server: moqui_host::Server,
    _serial: std::sync::MutexGuard<'static, ()>,
    _runtime: DirGuard,
    _cluster: DirGuard,
}

fn start(label: &str) -> Gate {
    let serial = SERIAL.lock().unwrap_or_else(|err| err.into_inner());
    let runtime = temp_dir(&format!("moqui-gate3-rt-{label}"));
    let cluster_dir = temp_dir(&format!("moqui-gate3-pg-{label}"));
    let runtime_guard = DirGuard(runtime.clone());
    let cluster_guard = DirGuard(cluster_dir.clone());
    std::fs::create_dir_all(runtime.join("conf")).unwrap();
    std::fs::create_dir_all(&cluster_dir).unwrap();
    std::fs::write(runtime.join("conf/MoquiDevConf.xml"), CONF).unwrap();
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
    let base =
        std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component")).unwrap();
    std::os::unix::fs::symlink(&base, runtime.join("base-component")).unwrap();

    let seed = load_data(&runtime, &framework, &cluster_dir, &LoadFlags::default()).unwrap();
    assert!(seed.enumeration > 0, "enumeration {}", seed.enumeration);
    let demo = base.join("webroot/data/WebrootSecurityDemoData.xml");
    let demo_flags = LoadFlags {
        location: Some(demo.display().to_string()),
        data_types: Some(vec!["demo".to_string()]),
        ..LoadFlags::default()
    };
    load_data(&runtime, &framework, &cluster_dir, &demo_flags).unwrap();
    // The default theme (style sheets, scripts, logo), the pagination text and the access to the applications list are component data, which a
    // load from the framework alone does not read.
    for name in ["WebrootThemeData.xml", "WebrootL10nData.xml", "WebrootSecurityData.xml"] {
        let flags = LoadFlags {
            location: Some(base.join("webroot/data").join(name).display().to_string()),
            ..LoadFlags::default()
        };
        load_data(&runtime, &framework, &cluster_dir, &flags).unwrap();
    }
    // The ADMIN group may view the System app, as the tools component's own security data says, and the
    // artifact group the screen shows has a member, two authorizations and a tarpit to list.
    let grant = runtime.join("system-app.xml");
    std::fs::write(
        &grant,
        r#"<entity-facade-xml type="seed">
            <moqui.security.ArtifactGroup artifactGroupId="SYSTEM_APP" description="System App (via root screen)"/>
            <moqui.security.ArtifactGroupMember artifactGroupId="SYSTEM_APP"
                artifactName="component://tools/screen/System.xml" artifactTypeEnumId="AT_XML_SCREEN"
                nameIsPattern="N" inheritAuthz="Y"/>
            <moqui.security.ArtifactAuthz artifactAuthzId="SYSTEM_APP_ADMIN" userGroupId="ADMIN"
                artifactGroupId="SYSTEM_APP" authzTypeEnumId="AUTHZT_ALWAYS" authzActionEnumId="AUTHZA_ALL"/>
            <moqui.security.ArtifactAuthz artifactAuthzId="SYSTEM_APP_ALL" userGroupId="ALL_USERS"
                artifactGroupId="SYSTEM_APP" authzTypeEnumId="AUTHZT_DENY" authzActionEnumId="AUTHZA_UPDATE"/>
            <moqui.security.ArtifactTarpit artifactTarpitId="SYSTEM_APP_TARPIT" userGroupId="ALL_USERS"
                artifactGroupId="SYSTEM_APP" maxHitsCount="100" maxHitsDuration="60" tarpitDuration="30"/>
            </entity-facade-xml>"#,
    )
    .unwrap();
    let grant_flags = LoadFlags {
        location: Some(grant.display().to_string()),
        ..LoadFlags::default()
    };
    load_data(&runtime, &framework, &cluster_dir, &grant_flags).unwrap();

    let bind = SocketAddr::from(([127, 0, 0, 1], 0));
    let server = start_server(&runtime, &framework, &cluster_dir, bind).unwrap();
    Gate {
        server,
        _serial: serial,
        _runtime: runtime_guard,
        _cluster: cluster_guard,
    }
}

#[test]
fn artifact_group_detail_renders_as_text_csv_xml_and_html_print() {
    let gate = start("modes");
    let address = gate.server.address();
    let john = basic("john.doe", "moqui");
    let get = |mode: &str| {
        let path = format!("/System/Security/ArtifactGroup/ArtifactGroupDetail.{mode}?artifactGroupId=SYSTEM_APP");
        let (status, headers, body) = exchange(address, &http_request("GET", &path, &[("Authorization", &john)], ""));
        assert_eq!(status, 200, "{mode}: {body}");
        assert!(!body.contains("Error rendering"), "{mode}: {body}");
        let content_type = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.clone());
        (content_type, body)
    };

    // text: fixed-width lines. A form-single line is the title right-aligned in 26 characters, a colon and
    // the value in 105. A form-list has column headers, a rule, and one line for each row.
    let (content_type, text) = get("text");
    assert_eq!(content_type.as_deref(), Some("text/plain;charset=UTF-8"));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], format!("{:>26}: {:<105}", "Artifact Group ID", "SYSTEM_APP"));
    assert_eq!(
        lines[1],
        format!("{:>26}: {:<105}", "Description", "System App (via root screen)")
    );
    let line = |wanted: &str| lines.contains(&wanted);
    assert!(line("Artifact Type     |Artifact Name     |Name Is Pattern   |Inherit Authz     |Filter Map        |Update            |Delete Link       "), "{text}");
    assert!(line("------------------+------------------+------------------+------------------+------------------+------------------+------------------"), "{text}");
    assert!(
        line("Screen            |component://tools/|N                 |Y                 |                  ||"),
        "{text}"
    );
    assert!(line("ID             |User Group     |Authz Type     |Action          |Authz Service Na|Entity Filter Se|Update          |Delete Link     "), "{text}");
    assert!(
        line("SYSTEM_APP_ADMI|Administrators ADMIN          |Always Allow (o|All             |                |||"),
        "{text}"
    );
    assert!(
        line("SYSTEM_APP_ALL |All Users (all ALL_USERS      |Deny (overrides|Update          |                |||"),
        "{text}"
    );
    assert!(line("User Group            |Max Hits Count       |Max Hits Duration    |Tarpit Duration      |Update               |Delete Link          "), "{text}");
    assert!(line("All Users (all users mALL_USERS             |100                  |60                   |30                   ||"), "{text}");

    // csv: a header line and a line for each row of each form-list, values quoted when they need it.
    let (content_type, csv) = get("csv");
    assert_eq!(content_type.as_deref(), Some("text/csv;charset=UTF-8"));
    assert_eq!(
        csv,
        "Artifact Type,Artifact Name,Name Is Pattern,Inherit Authz,Filter Map,Update,Delete Link\n\
         Screen,component://tools/screen/System.xml,N,Y,,\n\
         ID,User Group,Authz Type,Action,Authz Service Name,Entity Filter Sets,Update,Delete Link\n\
         SYSTEM_APP_ADMIN,Administrators (full access) - ADMIN,Always Allow (overrides Deny),All,,,\n\
         SYSTEM_APP_ALL,All Users (all users members by default) - ALL_USERS,Deny (overrides Allow),Update,,,\n\
         User Group,Max Hits Count,Max Hits Duration,Tarpit Duration,Update,Delete Link\n\
         All Users (all users members by default) - ALL_USERS,100,60,30,\n"
    );

    // xml: an element for each screen in the path, and for each form-list an element of entries whose
    // attributes are the displayed fields.
    let (content_type, xml) = get("xml");
    assert_eq!(content_type.as_deref(), Some("text/xml;charset=UTF-8"));
    assert_eq!(
        xml,
        "<webroot>\n<System>\n<Security>\n<ArtifactGroup>\n<ArtifactGroupDetail>\n\
         \x20   <ArtifactGroupMemberList>\n\
         \x20       <ArtifactGroupMemberListEntry artifactTypeEnumId=\"Screen\" artifactName=\"component://tools/screen/System.xml\" nameIsPattern=\"N\" inheritAuthz=\"Y\" filterMap=\"\" deleteLink=\" \"/>\n\
         \x20   </ArtifactGroupMemberList>\n\
         \x20   <ArtifactAuthzList>\n\
         \x20       <ArtifactAuthzListEntry artifactAuthzId=\"SYSTEM_APP_ADMIN\" userGroupId=\"Administrators (full access) - ADMIN\" authzTypeEnumId=\"Always Allow (overrides Deny)\" authzActionEnumId=\"All\" authzServiceName=\"\" entityFilterSets=\"\" deleteLink=\" \"/>\n\
         \x20       <ArtifactAuthzListEntry artifactAuthzId=\"SYSTEM_APP_ALL\" userGroupId=\"All Users (all users members by default) - ALL_USERS\" authzTypeEnumId=\"Deny (overrides Allow)\" authzActionEnumId=\"Update\" authzServiceName=\"\" entityFilterSets=\"\" deleteLink=\" \"/>\n\
         \x20   </ArtifactAuthzList>\n\
         \x20   <ArtifactTarpitList>\n\
         \x20       <ArtifactTarpitListEntry userGroupId=\"All Users (all users members by default) - ALL_USERS\" maxHitsCount=\"100\" maxHitsDuration=\"60\" tarpitDuration=\"30\" deleteLink=\" \"/>\n\
         \x20   </ArtifactTarpitList>\n\
         </ArtifactGroupDetail>\n</ArtifactGroup>\n</Security>\n</System>\n</webroot>\n"
    );

    // html-print: one document with print CSS. The form-single is a table of title and value, each form-list
    // is a table with a repeated header, and .xsl-fo, the older name of the mode, is the same body.
    let (content_type, print) = get("html-print");
    assert_eq!(content_type.as_deref(), Some("text/html;charset=UTF-8"));
    assert!(
        print.starts_with("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">"),
        "{print}"
    );
    assert_eq!(print.matches("<!DOCTYPE").count(), 1, "{print}");
    assert!(
        print.contains("@page letter-portrait { size: 8.5in 11in; margin: 0.25in; }"),
        "{print}"
    );
    assert!(
        print.contains("@page a4-landscape { size: 297mm 210mm; margin: 15mm; }"),
        "{print}"
    );
    assert!(print.contains("<body class=\"letter-portrait\">"), "{print}");
    assert!(print.ends_with("</body></html>\n"), "{print}");
    assert!(
        print.contains("<table class=\"field\"><tr><td class=\"field-title\">Artifact Group ID</td><td class=\"field-value\">SYSTEM_APP</td></tr></table>"),
        "{print}"
    );
    assert!(
        print.contains("<td class=\"field-value\">System App (via root screen)</td>"),
        "{print}"
    );
    assert_eq!(print.matches("<table class=\"form-list\">").count(), 3, "{print}");
    assert!(
        print.contains("<th><div style=\"text-align:left\">Artifact Type</div></th>"),
        "{print}"
    );
    assert!(
        print.contains("<td><div style=\"text-align:left\">component://tools/screen/System.xml</div></td>"),
        "{print}"
    );
    assert!(print.contains("SYSTEM_APP_ADMIN"), "{print}");
    assert!(print.contains("Always Allow (overrides Deny)"), "{print}");
    assert!(print.contains("All Users (all users members by default) - <a href=\"/System/Security/ArtifactGroup/ArtifactGroupDetail/userGroupDetail?artifactGroupId=SYSTEM_APP\">ALL_USERS</a>"), "{print}");
    let (content_type, xsl_fo) = get("xsl-fo");
    assert_eq!(content_type.as_deref(), Some("text/html;charset=UTF-8"));
    assert_eq!(xsl_fo, print);
    // The renderMode parameter takes either name, and the layout comes from layoutMaster.
    let path = "/System/Security/ArtifactGroup/ArtifactGroupDetail?artifactGroupId=SYSTEM_APP&renderMode=xsl-fo&layoutMaster=legal-landscape";
    let (status, _, landscape) = exchange(address, &http_request("GET", path, &[("Authorization", &john)], ""));
    assert_eq!(status, 200, "{landscape}");
    assert!(
        landscape.contains("@page { size: 14in 8.5in; margin: 0.25in;"),
        "{landscape}"
    );
    assert!(landscape.contains("<body class=\"legal-landscape\">"), "{landscape}");
}

#[test]
fn artifact_group_detail_renders_as_html_with_the_shared_client() {
    let gate = start("html");
    let address = gate.server.address();
    let john = basic("john.doe", "moqui");
    let path = "/System/Security/ArtifactGroup/ArtifactGroupDetail.html?artifactGroupId=SYSTEM_APP";
    let (status, headers, html) = exchange(address, &http_request("GET", path, &[("Authorization", &john)], ""));
    assert_eq!(status, 200, "{html}");
    assert_eq!(
        header(&headers, "content-type").as_deref(),
        Some("text/html;charset=UTF-8")
    );
    assert!(!html.contains("Error rendering"), "{html}");

    // the page shell: the shared style sheet, Font Awesome and the client script, with a version parameter
    assert!(html.starts_with("<!DOCTYPE html>\n<html lang=\"en-US\">"), "{html}");
    assert!(
        html.contains("<link rel=\"stylesheet\" href=\"/css/moqui-html.css?v="),
        "{html}"
    );
    assert!(
        html.contains("<link rel=\"stylesheet\" href=\"/libs/fontawesome/css/font-awesome.min.css?v="),
        "{html}"
    );
    assert!(html.contains("<script src=\"/js/moqui-html.js?v="), "{html}");
    assert!(
        html.contains("moqui-html.js?v=") && html.contains("\" defer></script>"),
        "{html}"
    );
    assert!(html.contains("<meta name=\"moquiSessionToken\" content=\""), "{html}");
    assert!(
        html.contains("<title>Artifact Groups - Artifact Group Detail</title>"),
        "{html}"
    );
    assert!(html.trim_end().ends_with("</body>\n</html>"), "{html}");

    // the form-single
    assert!(
        html.contains("<form name=\"ArtifactGroupForm\" id=\"ArtifactGroupForm\" method=\"post\" action=\"/System/Security/ArtifactGroup/ArtifactGroupDetail/updateArtifactGroup\">"),
        "{html}"
    );
    assert!(
        html.contains("<div class=\"field\"><label for=\"ArtifactGroupForm_artifactGroupId\">Artifact Group ID</label><div class=\"widget\"><span class=\"display\">SYSTEM_APP</span>"),
        "{html}"
    );
    assert!(
        html.contains("<input type=\"text\" id=\"ArtifactGroupForm_description\" name=\"description\" value=\"System App (via root screen)\" size=\"40\">"),
        "{html}"
    );
    // the form-lists: row forms outside the table, inputs that point to them, and the rows
    assert!(
        html.contains("<table class=\"form-list\" id=\"ArtifactAuthzList_table\">"),
        "{html}"
    );
    assert_eq!(html.matches("<table class=\"form-list\"").count(), 3, "{html}");
    assert!(
        html.contains("<form name=\"ArtifactAuthzList_0\" id=\"ArtifactAuthzList_0\" method=\"post\""),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"display\">SYSTEM_APP_ADMIN</span>"),
        "{html}"
    );
    assert!(
        html.contains(
            "<select name=\"authzTypeEnumId\" id=\"ArtifactAuthzList_authzTypeEnumId_0\" form=\"ArtifactAuthzList_0\">"
        ),
        "{html}"
    );
    assert!(
        html.contains("<option value=\"AUTHZT_ALWAYS\" selected>Always Allow (overrides Deny)</option>"),
        "{html}"
    );
    // dialogs are native dialog elements, and the forms they hold post the session token
    assert!(
        html.contains("<dialog id=\"AddAuthzDialog\" class=\"container-dialog\""),
        "{html}"
    );
    assert!(html.contains("data-dialog=\"AddAuthzDialog\""), "{html}");
    // link forms saved for the end of the page are markup and not text
    assert!(
        html.contains(
            "<form method=\"post\" action=\"/System/Security/ArtifactGroup/ArtifactGroupDetail/deleteArtifactAuthz\""
        ),
        "{html}"
    );
    assert!(!html.contains("&lt;form"), "{html}");
    // menus go to the navbar through an attribute, not a script
    assert!(html.contains("data-move-to=\"header-menus\""), "{html}");
    // no jQuery, Bootstrap, Select2, Moment or inline per-widget script
    let lower = html.to_ascii_lowercase();
    for word in [
        "jquery",
        "bootstrap",
        "select2",
        "selectivity",
        "moment",
        "datetimepicker",
        "<script>",
        "form-control",
        "$(",
    ] {
        assert!(!lower.contains(word), "{word} in {html}");
    }
    assert_eq!(html.matches("<script").count(), 1, "{html}");
}

#[test]
fn the_login_page_and_the_root_render_as_html_and_a_failed_login_comes_back_with_its_message() {
    let gate = start("login");
    let address = gate.server.address();
    let get = |path: &str, cookie: &str| {
        let headers: Vec<(&str, &str)> = if cookie.is_empty() {
            vec![]
        } else {
            vec![("Cookie", cookie)]
        };
        exchange(address, &http_request("GET", path, &headers, ""))
    };
    let (status, headers, login) = get("/Login", "");
    assert_eq!(status, 200, "{login}");
    assert_eq!(
        header(&headers, "content-type").as_deref(),
        Some("text/html;charset=UTF-8")
    );
    assert!(
        login.contains("<link rel=\"stylesheet\" href=\"/css/moqui-html.css?v="),
        "{login}"
    );
    assert!(
        login.contains("<form method=\"post\" action=\"/Login/login\" class=\"form-signin\" id=\"login_form\">"),
        "{login}"
    );
    assert!(
        login.contains("<input type=\"password\" name=\"password\" required"),
        "{login}"
    );
    assert!(
        login.contains("<nav class=\"tabs form-signin\" data-tab-hash=\"login\">"),
        "{login}"
    );
    let lower = login.to_ascii_lowercase();
    assert!(!lower.contains("jquery") && !lower.contains("bootstrap"), "{login}");
    let cookie = cookies_of(&headers);

    // the root goes to the sign-in page when nobody is signed in
    let (status, headers, _) = get("/", "");
    assert_eq!(status, 302);
    assert_eq!(header(&headers, "location").as_deref(), Some("/Login"));

    // a wrong password: the transition redirects back, and the next page shows the message once with the user name kept
    let body = "username=john.doe&password=wrong";
    let post = http_request(
        "POST",
        "/Login/login",
        &[
            ("Cookie", &cookie),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        body,
    );
    let (status, headers, text) = exchange(address, &post);
    assert_eq!(status, 302, "{text}");
    assert_eq!(header(&headers, "location").as_deref(), Some("/Login"));
    let cookie = cookies_of(&headers).max(cookie);
    let (status, _, again) = get("/Login", &cookie);
    assert_eq!(status, 200, "{again}");
    assert!(
        again.contains("<div class=\"alert\" data-type=\"danger\" role=\"alert\">"),
        "{again}"
    );
    assert!(
        again.contains("name=\"username\" type=\"text\" value=\"john.doe\""),
        "{again}"
    );
    let (_, _, third) = get("/Login", &cookie);
    assert!(!third.contains("class=\"alert\""), "{third}");

    // the same login as a multipart form: the fields are parameters, and a body over the cap is refused before any screen runs
    let multipart = |name: &str, extra: &str| {
        let body = format!(
            "--B0\r\nContent-Disposition: form-data; name=\"username\"\r\n\r\njohn.doe\r\n--B0\r\nContent-Disposition: form-data; name=\"password\"\r\n\r\nwrong\r\n{extra}--B0--\r\n"
        );
        let request = http_request(
            "POST",
            "/Login/login",
            &[
                ("Cookie", &cookie),
                ("Content-Type", "multipart/form-data; boundary=B0"),
            ],
            &body,
        );
        let _ = name;
        exchange(address, &request)
    };
    let (status, headers, text) = multipart("fields", "");
    assert_eq!(status, 302, "{text}");
    assert_eq!(header(&headers, "location").as_deref(), Some("/Login"));
    moqui_lib::env::set_property("webapp_max_upload", "1k");
    let big = format!(
        "--B0\r\nContent-Disposition: form-data; name=\"doc\"; filename=\"a.txt\"\r\n\r\n{}\r\n",
        "x".repeat(2000)
    );
    let (status, _, text) = multipart("big", &big);
    moqui_lib::env::clear_property("webapp_max_upload");
    assert_eq!(status, 413, "{text}");

    // the applications screen is html for a signed-in user, with the navbar the shell builds
    let john = basic("john.doe", "moqui");
    let (status, headers, apps) = exchange(address, &http_request("GET", "/apps", &[("Authorization", &john)], ""));
    assert_eq!(status, 200, "{apps}");
    // the page is on the screen history of the session, which the navbar's menu lists
    let session = cookies_of(&headers);
    let (_, _, apps_again) = exchange(
        address,
        &http_request("GET", "/apps", &[("Authorization", &john), ("Cookie", &session)], ""),
    );
    assert!(
        apps_again.contains("<li><a href=\"/apps/AppList\"><i class=\"fa fa-link\"></i> AppList</a></li>"),
        "{apps_again}"
    );
    assert!(apps.contains("<header><nav class=\"navbar\">"), "{apps}");
    assert!(apps.contains("<ul id=\"header-menus\"></ul>"), "{apps}");
    assert!(apps.contains("data-theme-toggle"), "{apps}");
    assert!(
        apps.contains("<a href=\"/apps/System\" data-btn=\"primary\" data-size=\"lg\" class=\"app-list-link\">"),
        "{apps}"
    );
    assert!(apps.contains("<div id=\"apps-footer-content\">"), "{apps}");
}

#[test]
fn static_files_are_served_with_validators_compression_and_no_way_out() {
    let gate = start("static");
    let address = gate.server.address();
    let get = |path: &str, headers: &[(&str, &str)]| exchange(address, &http_request("GET", path, headers, ""));
    let (status, headers, css) = get("/css/moqui-html.css", &[]);
    assert_eq!(status, 200);
    assert_eq!(
        header(&headers, "content-type").as_deref(),
        Some("text/css; charset=UTF-8")
    );
    assert_eq!(
        header(&headers, "cache-control").as_deref(),
        Some("public, max-age=3600, must-revalidate")
    );
    assert!(css.contains("--primary"), "{css}");
    let tag = header(&headers, "etag").unwrap();
    let (status, headers304, body) = get("/css/moqui-html.css", &[("If-None-Match", &tag)]);
    assert_eq!(status, 304);
    assert!(body.is_empty());
    assert_eq!(header(&headers304, "etag").as_deref(), Some(tag.as_str()));
    let (status, _, versioned) = get("/js/moqui-html.js?v=abc", &[]);
    assert_eq!(status, 200);
    assert!(versioned.contains("moqui"), "{versioned}");
    let (_, headers, _) = get("/js/moqui-html.js?v=abc", &[]);
    assert_eq!(
        header(&headers, "content-type").as_deref(),
        Some("text/javascript; charset=UTF-8")
    );
    assert_eq!(
        header(&headers, "cache-control").as_deref(),
        Some("public, max-age=31536000, immutable")
    );
    let (status, headers, _) = get("/favicon.ico", &[]);
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type").as_deref(), Some("image/x-icon"));
    // HEAD has the headers and no body
    let (status, headers, body) = exchange(address, &http_request("HEAD", "/css/moqui-html.css", &[], ""));
    assert_eq!(status, 200);
    assert!(body.is_empty());
    assert_eq!(header(&headers, "etag").as_deref(), Some(tag.as_str()));

    // gzip when the client takes it
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    stream
        .write_all(http_request("GET", "/css/moqui-html.css", &[("Accept-Encoding", "gzip")], "").as_bytes())
        .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|window| window == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    assert!(head.contains("content-encoding: gzip"), "{head}");
    assert!(raw.len() < css.len(), "{} compressed, {} plain", raw.len(), css.len());

    // nothing outside the directory, no hidden files, no screen or template sources
    for path in [
        "/css/../../../../etc/passwd",
        "/../../etc/passwd",
        "/css/%2e%2e/%2e%2e/%2e%2e/etc/passwd",
        "/css/..%2f..%2f..%2fetc%2fpasswd",
        "/css/..%5c..%5cetc%5cpasswd",
        "/.git/config",
        "/Login.html.fancy",
    ] {
        let (status, _, body) = get(path, &[]);
        assert!(status == 404 || status == 400, "{path}: {status} {body}");
        assert!(!body.contains("root:"), "{path}: {body}");
        assert!(!body.contains("<screen"), "{path}: {body}");
    }
}

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

/// The cookies a response sets as a `Cookie` header value.
fn cookies_of(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("set-cookie"))
        .filter_map(|(_, value)| value.split(';').next().map(str::to_string))
        .collect::<Vec<_>>()
        .join("; ")
}

fn basic(username: &str, password: &str) -> String {
    format!("Basic {}", base64(&format!("{username}:{password}").into_bytes()))
}

fn http_request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut out = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (name, value) in headers {
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    if !body.is_empty() || method.eq_ignore_ascii_case("POST") {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

fn exchange(address: SocketAddr, request: &str) -> (u16, Vec<(String, String)>, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 8192];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(count) => buf.extend_from_slice(&tmp[..count]),
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut || err.kind() == std::io::ErrorKind::WouldBlock => {
                if response_complete(&buf) {
                    break;
                }
                panic!("timed out after {} bytes: {}", buf.len(), String::from_utf8_lossy(&buf));
            }
            Err(err) => panic!("read failed after {} bytes: {err}", buf.len()),
        }
        if response_complete(&buf) {
            break;
        }
    }
    parse_response(&buf)
}

fn response_complete(buf: &[u8]) -> bool {
    let Some(split) = find_header_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..split]);
    let Some(length) = content_length(&head) else {
        return false;
    };
    buf.len() >= split + 4 + length
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &str) -> Option<usize> {
    for line in head.split("\r\n").skip(1) {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

fn parse_response(buf: &[u8]) -> (u16, Vec<(String, String)>, String) {
    let text = String::from_utf8_lossy(buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_ref(), ""));
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    (status, headers, body.to_string())
}

fn base64(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let chunk = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8) | (bytes[index + 2] as u32);
        push_base64(&mut out, chunk, 4);
        index += 3;
    }
    let rest = bytes.len() - index;
    if rest == 1 {
        push_base64(&mut out, (bytes[index] as u32) << 16, 2);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let chunk = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8);
        push_base64(&mut out, chunk, 3);
        out.push('=');
    }
    out
}

fn push_base64(out: &mut String, chunk: u32, count: usize) {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let shifts = [18, 12, 6, 0];
    for shift in shifts.into_iter().take(count) {
        let piece = ((chunk >> shift) & 63) as usize;
        out.push(ALPHABET[piece] as char);
    }
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{prefix}-{}-{nanos}", std::process::id());
    if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp").join(name)
    } else {
        std::env::temp_dir().join(name)
    }
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
