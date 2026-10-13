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

//! Gate 3, started: a real framework screen rendered in the text, csv and xml modes through the HTTP
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
    </screen-facade>
</moqui-conf>"#;

#[test]
fn artifact_group_detail_renders_as_text_csv_and_xml() {
    let runtime = temp_dir("moqui-gate3-rt");
    let cluster_dir = temp_dir("moqui-gate3-pg");
    let _runtime_guard = DirGuard(runtime.clone());
    let _cluster_guard = DirGuard(cluster_dir.clone());
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
    let address = server.address();
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

    drop(server);
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
