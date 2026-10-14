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

//! The seeded server of Gate 3 and the plain HTTP client its tests use, shared by `gate3.rs` and `gate3_browser.rs`.
//!
//! The fixture is `component://tools/screen/System/Security/ArtifactGroup/ArtifactGroupDetail.xml`. The tools screens are
//! not mounted under any app by the conf in this repository, so the setup adds the mount in a `conf/MoquiDevConf.xml` of its
//! temporary runtime, which is where a deployment would.

// Each test binary that includes this module uses part of it.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moqui_host::{load_data, start_server, LoadFlags};

pub mod cdp;

/// The fixture screen under the plain `html` root.
pub const FIXTURE: &str = "/System/Security/ArtifactGroup/ArtifactGroupDetail";

/// The conf of the test runtime. `companion` is the location of a screen with qjs and qvue text of its own, `widgets` and `static_list`
/// the screens of the browser smoke.
fn conf(companion: &str, widgets: &str, static_list: &str) -> String {
    format!(
        r#"<moqui-conf>
    <screen-facade>
        <screen location="component://webroot/screen/webroot.xml">
            <subscreens-item name="System" menu-title="System" location="component://tools/screen/System.xml"/>
            <subscreens-item name="Tools" menu-title="Tools" location="component://tools/screen/Tools.xml"/>
        </screen>
        <screen location="component://webroot/screen/webroot/apps.xml">
            <subscreens-item name="System" menu-title="System" location="component://tools/screen/System.xml"/>
            <subscreens-item name="GateCompanion" menu-title="Companion" menu-include="false" location="{companion}"/>
            <subscreens-item name="GateWidgets" menu-title="Widgets" menu-include="false" location="{widgets}"/>
            <subscreens-item name="GateStaticList" menu-title="Static List" menu-include="false" location="{static_list}"/>
        </screen>
    </screen-facade>
</moqui-conf>"#
    )
}

/// A screen with the three client forms: its widgets are the qvt template, and it has a qjs object and a qvue component file.
const COMPANION: &str = r#"<screen require-authentication="false">
    <widgets>
        <label text="Companion {{literal}} text" type="p"/>
        <render-mode>
            <text type="qjs"><![CDATA[define([], function () { return { data: function () { return { count: 1 }; } }; });]]></text>
            <text type="qvue"><![CDATA[<template><p>{{count}}</p></template>
<script>export default { data() { return { count: 2 }; } }</script>]]></text>
        </render-mode>
    </widgets>
</screen>"#;

/// The client features the browser smoke drives that the fixture does not have: click to edit, a background submit with a date, a
/// drop-down, a multiple drop-down (Tom Select in html) and a message, a form-list with the column picker, sort and pages, and Chart.js and Mermaid in the qvt text. Each transition
/// answers JSON with what it received, so the smoke reads what the server got from the response.
const WIDGETS: &str = r#"<screen require-authentication="false">
    <transition name="saveValue"><actions><script>ec.web.sendJsonResponse({saved: value || ''})</script></actions>
        <default-response type="none"/></transition>
    <transition name="saveNote"><actions><script>ec.web.sendJsonResponse({note: note || '', when: when || '', kind: kind || '', tags: tags || ''})</script></actions>
        <default-response type="none"/></transition>
    <transition name="checkCode"><actions><script>if (!/\d/.test(code || '')) ec.message.addValidationError(null, 'code', null, 'Code needs a digit', null)</script></actions>
        <default-response url="."/></transition>
    <transition name="formSelectColumns"><actions><script>ec.web.sendJsonResponse({columnsTree: columnsTree || ''})</script></actions>
        <default-response type="none"/></transition>
    <actions>
        <entity-find entity-name="moqui.basic.Enumeration" list="enumList">
            <search-form-inputs default-order-by="enumId"/></entity-find>
    </actions>
    <widgets>
        <label text="Gate widgets" type="h4"/>
        <editable id="GateEditable" text="Edit me" transition="saveValue" widget-type="text"/>
        <form-single name="NoteForm" transition="saveNote" background-submit="true" background-message="Note sent">
            <field name="note"><default-field title="Note"><text-line/></default-field></field>
            <field name="when"><default-field title="When"><date-time type="date"/></default-field></field>
            <field name="kind"><default-field title="Kind"><drop-down>
                <option key="A" text="Alpha"/><option key="B" text="Beta"/></drop-down></default-field></field>
            <field name="tags"><default-field title="Tags"><drop-down allow-multiple="true">
                <option key="X" text="Ex"/><option key="Y" text="Why"/></drop-down></default-field></field>
            <field name="submitButton"><default-field title="Send"><submit/></default-field></field>
        </form-single>
        <form-single name="CodeForm" transition="checkCode" background-submit="true" background-message="Code saved">
            <field name="code"><default-field title="Code"><text-line/></default-field></field>
            <field name="checkButton"><default-field title="Check"><submit/></default-field></field>
        </form-single>
        <form-list name="EnumList" list="enumList" select-columns="true">
            <field name="enumId"><header-field title="Enum ID" show-order-by="true"/><default-field><display/></default-field></field>
            <field name="description"><header-field show-order-by="true"/><default-field><display/></default-field></field>
            <field name="enumTypeId"><default-field><display/></default-field></field>
        </form-list>
        <render-mode><text type="qvt"><![CDATA[<div id="GateCharts"><m-chart :config="{type: 'bar', data: {labels: ['a', 'b'], datasets: [{label: 'n', data: [1, 2]}]}}" height="200px"></m-chart>
<m-mermaid height="200px">graph TD; GateA-->GateB</m-mermaid></div>]]></text></render-mode>
    </widgets>
</screen>"#;

/// A server-static screen with a long list: the static form-list's pages and sort run in the browser against `/actions`.
const STATIC_LIST: &str = r#"<screen require-authentication="false" server-static="qvt">
    <actions>
        <entity-find entity-name="moqui.basic.Enumeration" list="enumList">
            <search-form-inputs default-order-by="enumId"/></entity-find>
    </actions>
    <widgets>
        <label text="Enumerations of ${ec.user.username}" type="h4"/>
        <form-list name="EnumList" list="enumList">
            <field name="enumId"><header-field title="Enum ID" show-order-by="true"/><default-field><display/></default-field></field>
            <field name="description"><header-field show-order-by="true"/><default-field><display/></default-field></field>
        </form-list>
    </widgets>
</screen>"#;

/// The component of a deployment that marks the fixture screen server-static for qvt without editing the screen: a `screen-extend` file.
const STATIC_EXTEND: &str = r#"<screen-extend server-static="qvt"/>"#;

/// One embedded Postgres at a time: the tests set process-wide properties while they start.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The seeded server the gate tests share the setup of. Fields drop in order: the server stops before its directories go.
pub struct Gate {
    pub server: moqui_host::Server,
    /// The runtime directory the server reads, removed when the gate drops.
    pub runtime: PathBuf,
    /// The embedded cluster directory: `data` is the socket directory, `pgpass` the password.
    pub cluster_dir: PathBuf,
    _serial: std::sync::MutexGuard<'static, ()>,
    _runtime: DirGuard,
    _cluster: DirGuard,
}

pub fn start(label: &str) -> Gate {
    start_with(label, false)
}

/// [`start`] with one more `entity-facade-xml` document loaded after the grants, for the rows a test needs.
pub fn start_with_data(label: &str, data: &str) -> Gate {
    start_seeded(label, false, Some(data))
}

/// Load the seed (the framework files and each component's `data` directory), the demo users, and the grants of the fixture, then serve on an
/// ephemeral port. With `static_fixture` a `screen-extend` marks the fixture server-static for qvt.
pub fn start_with(label: &str, static_fixture: bool) -> Gate {
    start_seeded(label, static_fixture, None)
}

fn start_seeded(label: &str, static_fixture: bool, data: Option<&str>) -> Gate {
    let serial = SERIAL.lock().unwrap_or_else(|err| err.into_inner());
    let runtime = temp_dir(&format!("moqui-gate3-rt-{label}"));
    let cluster_dir = temp_dir(&format!("moqui-gate3-pg-{label}"));
    let runtime_guard = DirGuard(runtime.clone());
    let cluster_guard = DirGuard(cluster_dir.clone());
    std::fs::create_dir_all(runtime.join("conf")).unwrap();
    std::fs::create_dir_all(&cluster_dir).unwrap();
    let companion = runtime.join("GateCompanion.xml");
    std::fs::write(&companion, COMPANION).unwrap();
    let widgets = runtime.join("GateWidgets.xml");
    std::fs::write(&widgets, WIDGETS).unwrap();
    let static_list = runtime.join("GateStaticList.xml");
    std::fs::write(&static_list, STATIC_LIST).unwrap();
    std::fs::write(
        runtime.join("conf/MoquiDevConf.xml"),
        conf(
            &companion.display().to_string(),
            &widgets.display().to_string(),
            &static_list.display().to_string(),
        ),
    )
    .unwrap();
    if static_fixture {
        let component = runtime.join("component/gate-static");
        let extend = component.join("screen-extend/tools/screen/System/Security/ArtifactGroup");
        std::fs::create_dir_all(&extend).unwrap();
        std::fs::write(component.join("component.xml"), r#"<component name="gate-static"/>"#).unwrap();
        std::fs::write(extend.join("ArtifactGroupDetail.xml"), STATIC_EXTEND).unwrap();
    }
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
    let base =
        std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component")).unwrap();
    std::os::unix::fs::symlink(&base, runtime.join("base-component")).unwrap();

    // The seed load reads the data directory of each component too: the default themes, the pagination text, the access to the
    // applications list (webroot) and the System and Tools app groups (tools).
    let seed = load_data(&runtime, &framework, &cluster_dir, &LoadFlags::default()).unwrap();
    assert!(seed.enumeration > 0, "enumeration {}", seed.enumeration);
    let demo = base.join("webroot/data/WebrootSecurityDemoData.xml");
    let demo_flags = LoadFlags {
        location: Some(demo.display().to_string()),
        data_types: Some(vec!["demo".to_string()]),
        ..LoadFlags::default()
    };
    load_data(&runtime, &framework, &cluster_dir, &demo_flags).unwrap();
    // The artifact group the screen shows (SYSTEM_APP, from the tools component's own security data) gets a second
    // authorization and a tarpit to list, and its member says N for the pattern flag the tools data leaves empty.
    // The screens of the browser smoke read entities, so ADMIN gets them as an artifact group that passes its authorization on.
    let gate_members: String = [&widgets, &static_list]
        .iter()
        .map(|location| {
            format!(
                r#"<moqui.security.ArtifactGroupMember artifactGroupId="GATE_SCREENS" artifactName="{}"
                artifactTypeEnumId="AT_XML_SCREEN" nameIsPattern="N" inheritAuthz="Y"/>"#,
                location.display()
            )
        })
        .collect();
    let grant = runtime.join("system-app.xml");
    std::fs::write(
        &grant,
        r#"<entity-facade-xml type="seed">
            <moqui.security.ArtifactGroup artifactGroupId="GATE_SCREENS" description="Gate 3 browser screens"/>
            GATE_MEMBERS
            <moqui.security.ArtifactAuthz artifactAuthzId="GATE_SCREENS_ADMIN" userGroupId="ADMIN"
                artifactGroupId="GATE_SCREENS" authzTypeEnumId="AUTHZT_ALWAYS" authzActionEnumId="AUTHZA_ALL"/>
            <moqui.security.ArtifactGroupMember artifactGroupId="SYSTEM_APP"
                artifactName="component://tools/screen/System.xml" artifactTypeEnumId="AT_XML_SCREEN" nameIsPattern="N"/>
            <moqui.security.ArtifactAuthz artifactAuthzId="SYSTEM_APP_ALL" userGroupId="ALL_USERS"
                artifactGroupId="SYSTEM_APP" authzTypeEnumId="AUTHZT_DENY" authzActionEnumId="AUTHZA_UPDATE"/>
            <moqui.security.ArtifactTarpit artifactTarpitId="SYSTEM_APP_TARPIT" userGroupId="ALL_USERS"
                artifactGroupId="SYSTEM_APP" maxHitsCount="100" maxHitsDuration="60" tarpitDuration="30"/>
            </entity-facade-xml>"#
            .replace("GATE_MEMBERS", &gate_members),
    )
    .unwrap();
    let grant_flags = LoadFlags {
        location: Some(grant.display().to_string()),
        ..LoadFlags::default()
    };
    load_data(&runtime, &framework, &cluster_dir, &grant_flags).unwrap();
    if let Some(data) = data {
        let extra = runtime.join("test-data.xml");
        std::fs::write(&extra, data).unwrap();
        let extra_flags = LoadFlags {
            location: Some(extra.display().to_string()),
            ..LoadFlags::default()
        };
        load_data(&runtime, &framework, &cluster_dir, &extra_flags).unwrap();
    }

    // 0 binds an ephemeral cluster port, so a test run beside another server does not need 8443 free.
    moqui_lib::env::set_property("cluster_https_port", "0");
    let bind = SocketAddr::from(([127, 0, 0, 1], 0));
    let server = start_server(&runtime, &framework, &cluster_dir, bind).unwrap();
    Gate {
        server,
        runtime: runtime.clone(),
        cluster_dir: cluster_dir.clone(),
        _serial: serial,
        _runtime: runtime_guard,
        _cluster: cluster_guard,
    }
}

pub fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

/// The cookies a response sets as a `Cookie` header value.
pub fn cookies_of(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("set-cookie"))
        .filter_map(|(_, value)| value.split(';').next().map(str::to_string))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn basic(username: &str, password: &str) -> String {
    format!("Basic {}", base64(&format!("{username}:{password}").into_bytes()))
}

/// Sign in through the login form, as a browser does. Returns the session cookie.
pub fn sign_in(address: SocketAddr, username: &str, password: &str) -> String {
    let (_, headers, _) = exchange(address, &http_request("GET", "/Login", &[], ""));
    let cookie = cookies_of(&headers);
    let body = format!("username={username}&password={password}");
    let post = http_request(
        "POST",
        "/Login/login",
        &[
            ("Cookie", &cookie),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        &body,
    );
    let (status, headers, text) = exchange(address, &post);
    assert_eq!(status, 302, "{text}");
    merge_cookies(&cookie, &headers)
}

/// `cookie` with the cookies a response sets replacing those of the same name.
pub fn merge_cookies(cookie: &str, headers: &[(String, String)]) -> String {
    let mut jar: Vec<(String, String)> = cookie
        .split("; ")
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    for pair in cookies_of(headers).split("; ") {
        if let Some((name, value)) = pair.split_once('=') {
            match jar.iter_mut().find(|(known, _)| known == name) {
                Some(entry) => entry.1 = value.to_string(),
                None => jar.push((name.to_string(), value.to_string())),
            }
        }
    }
    jar.iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The value of `<meta name="moquiSessionToken" content="...">` in a page.
pub fn session_token(html: &str) -> Option<String> {
    let marker = "<meta name=\"moquiSessionToken\" content=\"";
    let start = html.find(marker)? + marker.len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_string())
}

pub fn http_request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
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

pub fn exchange(address: SocketAddr, request: &str) -> (u16, Vec<(String, String)>, String) {
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

pub fn temp_dir(prefix: &str) -> PathBuf {
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

pub struct DirGuard(pub PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
