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

//! SSH and SFTP on the seeded embedded Postgres, with the server as its own SFTP peer: the built-in sshd serves a
//! `dbresource://` root, the SFTP client logs in with a password and with a `UserSshKey`, the `moqui-sftp` services send
//! a SystemMessage and poll it back, the host key is trusted on first use and a changed key is refused, and the admin
//! shell runs for a user with `SSH_SHELL` and refuses one without.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::common::start_with_data;
use base64::Engine;
use moqui_core::sftp::SftpClient;
use moqui_lib::object::{to_plain_string, Value};
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::LineEnding;
use russh::keys::{HashAlg, PrivateKey};

/// Host key seeds: the server's, and the one it is replaced with.
const HOST_SEED: [u8; 32] = [7; 32];
const OTHER_SEED: [u8; 32] = [9; 32];
/// john.doe's login key.
const USER_SEED: [u8; 32] = [3; 32];

fn data(user_key: &PrivateKey) -> String {
    let public = user_key.public_key().to_openssh().unwrap();
    let fingerprint = user_key.public_key().fingerprint(HashAlg::Sha256).to_string();
    format!(
        r#"<entity-facade-xml type="seed">
    <moqui.security.UserSshKey sshKeyId="JOHN_KEY" userId="EX_JOHN_DOE" publicKey="{public}" fingerprint="{fingerprint}"
        fromDate="1265184000000"/>
    <!-- password=moqui, as john.doe -->
    <moqui.security.UserAccount userId="SFTP_ONLY" username="sftp.only" userFullName="SFTP Only"
        currentPassword="16ac58bbfa332c1c55bd98b53e60720bfa90d394" passwordHashType="SHA"/>
    <moqui.security.UserGroup userGroupId="SFTP_ONLY" description="SFTP only"/>
    <moqui.security.UserGroupMember userGroupId="SFTP_ONLY" userId="SFTP_ONLY" fromDate="1265184000000"/>
    <moqui.security.UserGroupPermission userGroupId="SFTP_ONLY" userPermissionId="SSH_SFTP" fromDate="1265184000000"/>
    <moqui.security.UserAccount userId="NO_SSH" username="no.ssh" userFullName="No SSH"
        currentPassword="16ac58bbfa332c1c55bd98b53e60720bfa90d394" passwordHashType="SHA"/>

    <moqui.service.message.SystemMessageType systemMessageTypeId="SshOut" description="Out by SFTP"
        sendServiceName="org.moqui.sftp.SftpMessageServices.send#SystemMessageSftp"
        sendPath="/outbox/${{systemMessageTypeId}}"/>
    <moqui.service.message.SystemMessageType systemMessageTypeId="SshIn" description="In by SFTP"
        consumeServiceName="org.moqui.impl.SystemMessageServices.reset#SystemMessageInError"
        receivePath="/outbox/SshOut" receiveFilePattern=".*" receiveResponseEnumId="MsgRrMove"
        receiveMovePath="/outbox/done"/>
</entity-facade-xml>"#
    )
}

fn key(seed: [u8; 32]) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&seed))
}

fn sync(name: &str, parameters: &[(&str, Value)]) -> (BTreeMap<String, Value>, String) {
    let parameters: BTreeMap<String, Value> = parameters
        .iter()
        .map(|(name, value)| ((*name).to_string(), value.clone()))
        .collect();
    let result = moqui_core::service::sync(name, &parameters).unwrap_or_default();
    let errors = moqui_core::context::ExecutionContext::with_current(|ctx| {
        let text = ctx.message().errors().join("\n");
        ctx.message().clear();
        text
    })
    .unwrap_or_default();
    (result, errors)
}

/// Run one command over `exec` and return the output and the exit status.
fn exec(address: std::net::SocketAddr, username: &str, password: &str, command: &str) -> Result<(String, u32), String> {
    struct Accept;
    impl russh::client::Handler for Accept {
        type Error = russh::Error;
        async fn check_server_key(&mut self, _key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }
    let (username, password, command) = (username.to_string(), password.to_string(), command.to_string());
    moqui_host::edge::block_on(async move {
        let config = Arc::new(russh::client::Config::default());
        let mut handle = russh::client::connect(config, address, Accept)
            .await
            .map_err(|err| err.to_string())?;
        let auth = handle
            .authenticate_password(username, password)
            .await
            .map_err(|err| err.to_string())?;
        if !auth.success() {
            return Err("login refused".to_string());
        }
        let mut channel = handle.channel_open_session().await.map_err(|err| err.to_string())?;
        channel.exec(true, command).await.map_err(|err| err.to_string())?;
        let mut out = Vec::new();
        let mut status = None;
        while let Some(message) = channel.wait().await {
            match message {
                russh::ChannelMsg::Data { data } => out.extend_from_slice(&data),
                russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                russh::ChannelMsg::Failure => return Err("channel request refused".to_string()),
                _ => {}
            }
        }
        Ok((String::from_utf8_lossy(&out).into_owned(), status.unwrap_or(255)))
    })
    .map_err(|err| err.to_string())?
}

#[test]
fn sftp_and_the_admin_shell_run_on_the_built_in_server() {
    let user_key = key(USER_SEED);
    let seed = base64::engine::general_purpose::STANDARD.encode(HOST_SEED);
    moqui_lib::env::set_property("ssh_port", "0");
    moqui_lib::env::set_property("ssh_host_key", &seed);
    moqui_lib::env::set_property("ssh_sftp_root", "dbresource://sftp");
    moqui_lib::env::set_property("cluster_transport", "ssh");
    let gate = start_with_data("ssh", &data(&user_key));
    let address = gate.server.ssh_address().expect("the SSH server listens");
    let host = format!("127.0.0.1:{}", address.port());
    let known_hosts = gate.runtime.join("conf/ssh_known_hosts");
    let factory = Arc::clone(gate.server.factory());
    let context = factory.new_context();
    let _entered = context.enter();
    moqui_core::user::internal_login_user("john.doe").unwrap();
    // The test reads the resource and message rows directly; the services check their own authorization.
    let authz = context.disable_authz();

    // Password login; the first connection records the host key.
    let mut client = SftpClient::new(&host, "john.doe", None).unwrap();
    client.password("moqui").connect().unwrap();
    assert!(std::fs::read_to_string(&known_hosts)
        .unwrap()
        .contains(&format!("[127.0.0.1]:{}", address.port())));
    client.mkdirs("/drop/deep").unwrap();
    assert_eq!(
        client
            .put_text("/drop", "a.txt", "caf\u{e9}", "ISO-8859-1", false)
            .unwrap(),
        "/drop/a.txt"
    );
    client.put_bytes("/drop/deep/b.bin", "", &[0, 1, 2, 255]).unwrap();
    assert_eq!(client.get_text("/drop/a.txt", "ISO-8859-1").unwrap(), "caf\u{e9}");
    assert_eq!(client.get_binary("/drop/deep/b.bin").unwrap(), [0, 1, 2, 255]);
    let listed = client.ls("/drop").unwrap();
    let names: Vec<&str> = listed.iter().map(|(_, entry)| entry.name.as_str()).collect();
    assert_eq!(names, ["a.txt", "deep"]);
    assert_eq!(client.ls_files("/drop").unwrap(), ["a.txt"]);
    assert_eq!(client.rename_file("/drop/a.txt", "c.txt").unwrap(), "/drop/c.txt");
    assert_eq!(client.move_file("/drop/c.txt", "/kept", true).unwrap(), "/kept/c.txt");
    client.rm("/kept/c.txt").unwrap();
    assert!(client.ls_files("/kept").unwrap().is_empty());
    // A path that climbs out of the root is refused.
    assert!(client.get_binary("/../../etc/passwd").is_err());
    // The file landed in the database resource tree.
    let stored = factory
        .resource()
        .stat("dbresource://sftp/drop/deep/b.bin")
        .unwrap()
        .unwrap();
    assert_eq!(stored.size, 4);
    // Streams, as Java's openStream and putStream.
    let handle = client.put_stream("/drop/s.txt").unwrap();
    client.write_chunk(handle, b"one ").unwrap();
    client.write_chunk(handle, b"two").unwrap();
    client.close_handle(handle).unwrap();
    assert_eq!(client.get_binary("/drop/s.txt").unwrap(), b"one two");
    // The send below puts into this directory; without it the sendPath would be the file's own name, as in Java.
    client.mkdirs("/outbox/SshOut").unwrap();
    client.close();

    // Key login with the user's UserSshKey, and a key that is not one is refused.
    let pem = user_key.to_openssh(LineEnding::LF).unwrap().to_string();
    let mut keyed = SftpClient::new(&host, "john.doe", None).unwrap();
    keyed.public_key_pkcs8(&pem, None).connect().unwrap();
    assert_eq!(keyed.ls_files("/drop").unwrap(), ["s.txt"]);
    keyed.close();
    let stranger = key(OTHER_SEED).to_openssh(LineEnding::LF).unwrap().to_string();
    let mut refused = SftpClient::new(&host, "john.doe", None).unwrap();
    assert!(refused.public_key_pkcs8(&stranger, None).connect().is_err());
    // A wrong password and a user without an SSH permission are refused.
    assert!(SftpClient::new(&host, "john.doe", None)
        .unwrap()
        .password("wrong")
        .connect()
        .is_err());
    assert!(SftpClient::new(&host, "no.ssh", None)
        .unwrap()
        .password("moqui")
        .connect()
        .is_err());

    // The moqui-sftp services: a queued message goes out by SFTP, and the poll brings it back in.
    let (_, errors) = sync(
        "store#moqui.service.message.SystemMessageRemote",
        &[
            ("systemMessageRemoteId", Value::string("SshSelf")),
            ("sendUrl", Value::string(&host)),
            ("username", Value::string("john.doe")),
            ("password", Value::string("moqui")),
        ],
    );
    assert_eq!(errors, "");
    let (queued, errors) = sync(
        "org.moqui.impl.SystemMessageServices.queue#SystemMessage",
        &[
            ("systemMessageTypeId", Value::string("SshOut")),
            ("systemMessageRemoteId", Value::string("SshSelf")),
            ("messageText", Value::string("{\"order\": 1}")),
            ("sendNow", Value::Bool(false)),
        ],
    );
    assert_eq!(errors, "");
    let sent_id = to_plain_string(queued.get("systemMessageId").unwrap());
    let (_, errors) = sync(
        "org.moqui.impl.SystemMessageServices.send#ProducedSystemMessage",
        &[("systemMessageId", Value::string(&sent_id))],
    );
    assert_eq!(errors, "");
    let row = |id: &str, field: &str| {
        factory
            .entity()
            .unwrap()
            .find("moqui.service.message.SystemMessage")
            .unwrap()
            .disable_authz()
            .eq("systemMessageId", Value::string(id))
            .one()
            .unwrap()
            .and_then(|row| row.get(field).map(to_plain_string))
            .unwrap_or_default()
    };
    let send_errors: Vec<String> = factory
        .entity()
        .unwrap()
        .find("moqui.service.message.SystemMessageError")
        .unwrap()
        .disable_authz()
        .eq("systemMessageId", Value::string(&sent_id))
        .list()
        .unwrap()
        .values()
        .iter()
        .map(|error| error.get("errorText").map(to_plain_string).unwrap_or_default())
        .collect();
    assert_eq!(row(&sent_id, "statusId"), "SmsgSent", "{send_errors:?}");
    assert_eq!(row(&sent_id, "remoteMessageId"), sent_id);
    assert!(factory
        .resource()
        .stat(&format!("dbresource://sftp/outbox/SshOut/{sent_id}"))
        .unwrap()
        .is_some());

    let (polled, errors) = sync(
        "org.moqui.sftp.SftpMessageServices.poll#SystemMessageSftp",
        &[
            ("systemMessageTypeId", Value::string("SshIn")),
            ("systemMessageRemoteId", Value::string("SshSelf")),
        ],
    );
    assert_eq!(errors, "");
    assert_eq!(polled.get("fileCount").map(to_plain_string).as_deref(), Some("1"));
    let received = factory
        .entity()
        .unwrap()
        .find("moqui.service.message.SystemMessage")
        .unwrap()
        .disable_authz()
        .eq("systemMessageTypeId", Value::string("SshIn"))
        .list()
        .unwrap();
    assert_eq!(received.values().len(), 1);
    assert_eq!(
        received.values()[0].get("messageText").map(to_plain_string).as_deref(),
        Some("{\"order\": 1}")
    );
    assert!(factory
        .resource()
        .stat(&format!("dbresource://sftp/outbox/done/{sent_id}"))
        .unwrap()
        .is_some());
    assert!(factory
        .resource()
        .stat(&format!("dbresource://sftp/outbox/SshOut/{sent_id}"))
        .unwrap()
        .is_none());

    // A pinned fingerprint must match.
    let real = key(HOST_SEED).public_key().fingerprint(HashAlg::Sha256).to_string();
    let mut pinned = SftpClient::new(&host, "john.doe", None).unwrap();
    pinned.password("moqui").host_key_fingerprint(&real).connect().unwrap();
    pinned.close();
    let wrong = key(OTHER_SEED).public_key().fingerprint(HashAlg::Sha256).to_string();
    let mut pinned = SftpClient::new(&host, "john.doe", None).unwrap();
    let err = pinned
        .password("moqui")
        .host_key_fingerprint(&wrong)
        .connect()
        .err()
        .unwrap();
    assert!(err.message().contains("not the pinned"), "{}", err.message());

    // The admin shell: exec as john.doe (SSH_SHELL), and refused for a user with SFTP only.
    let (out, status) = exec(address, "john.doe", "moqui", "status").unwrap();
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("user john.doe"), "{out}");
    let (out, status) = exec(address, "john.doe", "moqui", "js 6 * 7").unwrap();
    assert_eq!((out.trim(), status), ("42", 0));
    let (out, status) = exec(
        address,
        "john.doe",
        "moqui",
        "js const n = ec.user.username; return n + '!'",
    )
    .unwrap();
    assert_eq!((out.trim(), status), ("\"john.doe!\"", 0));
    let (out, status) = exec(
        address,
        "john.doe",
        "moqui",
        "entity moqui.security.UserAccount {\"username\": \"sftp.only\"}",
    )
    .unwrap();
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("SFTP_ONLY"), "{out}");
    let (out, status) = exec(address, "john.doe", "moqui", "service no.such#Service").unwrap();
    assert_eq!(status, 1, "{out}");
    let refused = exec(address, "sftp.only", "moqui", "status");
    assert_eq!(refused, Err("channel request refused".to_string()));
    let mut sftp_only = SftpClient::new(&host, "sftp.only", None).unwrap();
    sftp_only.password("moqui").connect().unwrap();
    sftp_only.close();

    // A server whose key changed is refused: the known_hosts line now names another key.
    let text = std::fs::read_to_string(&known_hosts).unwrap();
    let other = key(OTHER_SEED).public_key().to_openssh().unwrap();
    let marker = format!("[127.0.0.1]:{}", address.port());
    let changed: String = text
        .lines()
        .map(|line| {
            if line.starts_with(&marker) {
                format!("{marker} {other}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&known_hosts, changed + "\n").unwrap();
    let started = Instant::now();
    let err = SftpClient::new(&host, "john.doe", None)
        .unwrap()
        .password("moqui")
        .connect()
        .err()
        .unwrap();
    assert!(err.message().contains("changed"), "{}", err.message());
    assert!(started.elapsed() < Duration::from_secs(30));
    // The SSH cluster channel: this member registered ssh://its address with its host key; a peer's post arrives
    // through the channel and is applied as an HTTPS post would be.
    let base_url = format!("ssh://{address}");
    let member = factory
        .entity()
        .unwrap()
        .find("moqui.server.ClusterMember")
        .unwrap()
        .disable_authz()
        .list()
        .unwrap();
    let row = member.values().first().expect("the member row");
    assert_eq!(
        row.get("clusterBaseUrl").map(to_plain_string).as_deref(),
        Some(base_url.as_str())
    );
    assert_eq!(
        row.get("sshHostKeyFingerprint").map(to_plain_string),
        Some(real.clone())
    );
    let sender = moqui_host::ssh::cluster::sender(&factory).unwrap();
    let payload = br#"{"origin_id": "peer-1", "message_id": "m-1", "body": {"entityName": "moqui.basic.Enumeration"}}"#;
    let before = gate.server.cluster_posts().len();
    sender.post(&base_url, "entity-cache-invalidate", payload).unwrap();
    sender.post(&base_url, "entity-cache-invalidate", payload).unwrap();
    let posts = gate.server.cluster_posts();
    assert_eq!(posts.len(), before + 2, "{posts:?}");
    assert_eq!(posts.last().map(String::as_str), Some("entity-cache-invalidate"));
    // An unknown topic is refused by the peer, and a key that is not the cluster key cannot log in as the cluster.
    assert!(sender.post(&base_url, "no-such-topic", payload).is_err());
    let stranger = Arc::new(key(OTHER_SEED));
    let refused = moqui_host::edge::block_on(async move {
        struct Accept;
        impl russh::client::Handler for Accept {
            type Error = russh::Error;
            async fn check_server_key(
                &mut self,
                _key: &russh::keys::PublicKeyOrCertificate,
            ) -> Result<bool, Self::Error> {
                Ok(true)
            }
        }
        let config = Arc::new(russh::client::Config::default());
        let mut handle = russh::client::connect(config, address, Accept).await.unwrap();
        let key = russh::keys::PrivateKeyWithHashAlg::new(stranger, None);
        handle
            .authenticate_publickey("moqui-cluster", key)
            .await
            .unwrap()
            .success()
    })
    .unwrap();
    assert!(!refused, "a stranger key logged in as the cluster");

    drop(authz);
    drop(_entered);
    drop(context);
    moqui_lib::env::set_property("ssh_port", "");
    moqui_lib::env::set_property("cluster_transport", "");
}
