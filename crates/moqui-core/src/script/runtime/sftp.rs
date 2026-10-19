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

//! `SftpClient` for scripts: the Groovy `new org.moqui.sftp.SftpClient(host, username)` is `new SftpClient(host,
//! username)`, a global like `EdiHandler`. The builder calls (`password`, `publicKeyPkcs8`, `preserveAttributes`,
//! `connect`) and `put`/`rm` return the client, so calls chain as in Java. Bytes are `Uint8Array`s. `openStream` and
//! `putStream` return small stream objects, since a script has no Java streams. The Java-object overloads
//! (`publicKey(PrivateKey, PublicKey)`, `publicKey(KeyProvider...)`, `getInternalClient`) have no script form and throw.
//! A client the script did not close is closed when it is collected, with an error in the log, as Java's `finalize`.

use crate::sftp::{SftpClient, SftpKind};

use super::*;

const SFTP_CLIENT_JS: &str = r#"(function (make) {
    function SftpClient(host, username, port) {
        this.client = make(host == null ? "" : String(host), username == null ? "" : String(username),
            port == null ? null : Number(port));
    }
    ["password", "publicKeyPkcs8", "preserveAttributes", "hostKeyFingerprint", "connect"].forEach(function (name) {
        SftpClient.prototype[name] = function () { this.client[name].apply(this.client, arguments); return this; };
    });
    ["lsFiles", "ls", "getBinary", "getText", "moveFile", "renameFile", "rename", "close", "isConnected"]
        .forEach(function (name) {
            SftpClient.prototype[name] = function () { return this.client[name].apply(this.client, arguments); };
        });
    SftpClient.prototype.put = function (path, filename, data, charset, createDir) {
        if (data != null && typeof data === "object" && !(data instanceof Uint8Array) && !(data instanceof ArrayBuffer)
                && typeof data.getLocation === "function") data = { resourceLocation: String(data.getLocation()) };
        this.client.put(path, filename, data, charset == null ? null : String(charset), !!createDir);
        return this;
    };
    SftpClient.prototype.rm = function (path) { this.client.rm(path); return this; };
    SftpClient.prototype.openStream = function (path) {
        var client = this.client, handle = client.openStream(path);
        return {
            read: function (max) { return client.readChunk(handle, max == null ? 65536 : max); },
            readAllBytes: function () { return client.readAll(handle); },
            readText: function (charset) { return client.readAllText(handle, charset == null ? null : String(charset)); },
            close: function () { client.closeStream(handle); }
        };
    };
    SftpClient.prototype.putStream = function (path) {
        var client = this.client, handle = client.putStream(path);
        return {
            write: function (data, charset) { client.writeChunk(handle, data, charset == null ? null : String(charset)); },
            close: function () { client.closeStream(handle); }
        };
    };
    var noScriptForm = function (name) {
        return function () {
            throw new Error("SftpClient." + name + " takes Java key objects, which a script does not have; use publicKeyPkcs8(privateKeyPem, publicKey)");
        };
    };
    SftpClient.prototype.publicKey = noScriptForm("publicKey");
    SftpClient.prototype.getInternalClient = noScriptForm("getInternalClient");
    SftpClient.SFTP_PORT = 115;
    SftpClient.SSH_PORT = 22;
    return SftpClient;
})"#;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostSftpClient {
    #[qjs(skip_trace)]
    inner: RefCell<SftpClient>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostSftpClient {
    fn password<'js>(&self, ctx: Ctx<'js>, password: Opt<JsValue<'js>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let password = text_or_empty(&ctx, password.0.as_ref())?;
            self.inner.borrow_mut().password(&password);
            Ok(())
        })
    }

    fn public_key_pkcs8<'js>(
        &self,
        ctx: Ctx<'js>,
        private_key: JsValue<'js>,
        public_key: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let private_key = text_or_empty(&ctx, Some(&private_key))?;
            let public_key = text_or_empty(&ctx, public_key.0.as_ref())?;
            self.inner
                .borrow_mut()
                .public_key_pkcs8(&private_key, Some(public_key.as_str()));
            Ok(())
        })
    }

    fn preserve_attributes(&self, ctx: Ctx<'_>, preserve: bool) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            self.inner.borrow_mut().preserve_attributes(preserve);
            Ok(())
        })
    }

    fn host_key_fingerprint(&self, ctx: Ctx<'_>, fingerprint: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            self.inner.borrow_mut().host_key_fingerprint(&fingerprint);
            Ok(())
        })
    }

    fn connect(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            self.inner
                .borrow_mut()
                .connect()
                .map(|_| ())
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn is_connected(&self, ctx: Ctx<'_>) -> rquickjs::Result<bool> {
        host_guard(&ctx, || Ok(self.inner.borrow().is_connected()))
    }

    fn ls_files<'js>(&self, ctx: Ctx<'js>, path: String) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let names = self
                .inner
                .borrow_mut()
                .ls_files(&path)
                .map_err(|err| throw_lib(&ctx, &err))?;
            host_to_js(&ctx, Value::List(names.into_iter().map(Value::String).collect()))
        })
    }

    /// Maps of `name, path, isFile, size, lastAccess, lastModified`, as Java's `ls`.
    fn ls<'js>(&self, ctx: Ctx<'js>, path: String) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let entries = self.inner.borrow_mut().ls(&path).map_err(|err| throw_lib(&ctx, &err))?;
            let list = entries
                .into_iter()
                .map(|(full, entry)| {
                    let mut map = indexmap::IndexMap::new();
                    let attributes = entry.attributes;
                    map.insert("name".to_string(), Value::String(entry.name));
                    map.insert("path".to_string(), Value::String(full));
                    map.insert("isFile".to_string(), Value::Bool(attributes.kind == SftpKind::File));
                    map.insert(
                        "size".to_string(),
                        Value::I64(attributes.size.and_then(|size| i64::try_from(size).ok()).unwrap_or(0)),
                    );
                    map.insert(
                        "lastAccess".to_string(),
                        Value::I64(i64::from(attributes.access_time.unwrap_or(0))),
                    );
                    map.insert(
                        "lastModified".to_string(),
                        Value::I64(i64::from(attributes.modified_time.unwrap_or(0))),
                    );
                    Value::Map(map)
                })
                .collect();
            host_to_js(&ctx, Value::List(list))
        })
    }

    fn get_binary<'js>(&self, ctx: Ctx<'js>, path: String) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let bytes = self
                .inner
                .borrow_mut()
                .get_binary(&path)
                .map_err(|err| throw_lib(&ctx, &err))?;
            host_to_js(&ctx, Value::Bytes(bytes))
        })
    }

    fn get_text<'js>(&self, ctx: Ctx<'js>, path: String, charset: Opt<JsValue<'js>>) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            let charset = text_or_empty(&ctx, charset.0.as_ref())?;
            self.inner
                .borrow_mut()
                .get_text(&path, &charset)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    /// Text in a charset, or bytes (a `Uint8Array`, an `ArrayBuffer`, an uploaded file, or a resource location).
    fn put<'js>(
        &self,
        ctx: Ctx<'js>,
        path: String,
        filename: String,
        data: JsValue<'js>,
        charset: Opt<JsValue<'js>>,
        create_dir: Opt<bool>,
    ) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let create_dir = create_dir.0.unwrap_or(false);
            if let Some(text) = data.as_string() {
                let text = text.to_string()?;
                let charset = text_or_empty(&ctx, charset.0.as_ref())?;
                return self
                    .inner
                    .borrow_mut()
                    .put_text(&path, &filename, &text, &charset, create_dir)
                    .map(|_| ())
                    .map_err(|err| throw_lib(&ctx, &err));
            }
            let bytes = data_bytes(&ctx, &data)?;
            let mut client = self.inner.borrow_mut();
            if create_dir {
                let parent = path.trim_end_matches('/').rsplit_once('/').map_or("", |(dir, _)| dir);
                if !parent.is_empty() {
                    client.mkdirs(parent).map_err(|err| throw_lib(&ctx, &err))?;
                }
            }
            client
                .put_bytes(&path, &filename, &bytes)
                .map(|_| ())
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn move_file<'js>(
        &self,
        ctx: Ctx<'js>,
        file_path: String,
        dest_dir: String,
        create_dir: Opt<bool>,
    ) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            self.inner
                .borrow_mut()
                .move_file(&file_path, &dest_dir, create_dir.0.unwrap_or(true))
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn rename_file(&self, ctx: Ctx<'_>, file_path: String, new_name: String) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            self.inner
                .borrow_mut()
                .rename_file(&file_path, &new_name)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn rename(&self, ctx: Ctx<'_>, from: String, to: String) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            self.inner
                .borrow_mut()
                .rename(&from, &to)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn rm(&self, ctx: Ctx<'_>, path: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            self.inner.borrow_mut().rm(&path).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn open_stream(&self, ctx: Ctx<'_>, path: String) -> rquickjs::Result<f64> {
        host_guard(&ctx, || {
            let handle = self
                .inner
                .borrow_mut()
                .open_stream(&path)
                .map_err(|err| throw_lib(&ctx, &err))?;
            #[allow(clippy::cast_precision_loss)]
            Ok(handle as f64)
        })
    }

    fn put_stream(&self, ctx: Ctx<'_>, path: String) -> rquickjs::Result<f64> {
        host_guard(&ctx, || {
            let handle = self
                .inner
                .borrow_mut()
                .put_stream(&path)
                .map_err(|err| throw_lib(&ctx, &err))?;
            #[allow(clippy::cast_precision_loss)]
            Ok(handle as f64)
        })
    }

    fn read_chunk<'js>(&self, ctx: Ctx<'js>, handle: f64, max: f64) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let bytes = self
                .inner
                .borrow_mut()
                .read_chunk(stream_handle(handle), stream_size(max))
                .map_err(|err| throw_lib(&ctx, &err))?;
            host_to_js(&ctx, Value::Bytes(bytes))
        })
    }

    fn read_all<'js>(&self, ctx: Ctx<'js>, handle: f64) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let bytes = self.read_rest(handle).map_err(|err| throw_lib(&ctx, &err))?;
            host_to_js(&ctx, Value::Bytes(bytes))
        })
    }

    fn read_all_text<'js>(&self, ctx: Ctx<'js>, handle: f64, charset: Opt<JsValue<'js>>) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            let charset = text_or_empty(&ctx, charset.0.as_ref())?;
            let encoding = crate::sftp::encoding_for(&charset).map_err(|err| throw_lib(&ctx, &err))?;
            let bytes = self.read_rest(handle).map_err(|err| throw_lib(&ctx, &err))?;
            Ok(crate::sftp::decode(encoding, &bytes))
        })
    }

    fn write_chunk<'js>(
        &self,
        ctx: Ctx<'js>,
        handle: f64,
        data: JsValue<'js>,
        charset: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let bytes = if let Some(text) = data.as_string() {
                let charset = text_or_empty(&ctx, charset.0.as_ref())?;
                let encoding = crate::sftp::encoding_for(&charset).map_err(|err| throw_lib(&ctx, &err))?;
                crate::sftp::encode(encoding, &text.to_string()?)
            } else {
                data_bytes(&ctx, &data)?
            };
            self.inner
                .borrow_mut()
                .write_chunk(stream_handle(handle), &bytes)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn close_stream(&self, ctx: Ctx<'_>, handle: f64) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            self.inner
                .borrow_mut()
                .close_handle(stream_handle(handle))
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn close(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            self.inner.borrow_mut().close();
            Ok(())
        })
    }
}

impl HostSftpClient {
    fn read_rest(&self, handle: f64) -> Result<Vec<u8>, moqui_lib::error::Error> {
        let mut client = self.inner.borrow_mut();
        let mut out = Vec::new();
        loop {
            let chunk = client.read_chunk(stream_handle(handle), 1 << 16)?;
            if chunk.is_empty() {
                return Ok(out);
            }
            out.extend_from_slice(&chunk);
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn stream_handle(handle: f64) -> u64 {
    handle.max(0.0) as u64
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn stream_size(max: f64) -> usize {
    if max.is_finite() && max >= 1.0 {
        (max as usize).min(1 << 20)
    } else {
        1 << 16
    }
}

/// Bytes from a `Uint8Array`, an `ArrayBuffer`, an uploaded file, or a resource location string.
fn data_bytes(ctx: &Ctx<'_>, data: &JsValue<'_>) -> rquickjs::Result<Vec<u8>> {
    if let Some(object) = data.as_object() {
        if let Some(bytes) = super::convert::js_bytes(object) {
            return Ok(bytes);
        }
        if let Some(upload) = object.as_class::<super::upload::HostUpload>() {
            return upload
                .borrow()
                .inner
                .read_bytes()
                .map_err(|err| Exception::throw_message(ctx, err.message()));
        }
        if let Ok(location) = object.get::<_, String>("resourceLocation") {
            let found = with_ec(ctx, |ec| ec.factory().resource().get(&location))?;
            return found
                .and_then(|resource| resource.read_bytes())
                .map_err(|err| Exception::throw_message(ctx, err.message()));
        }
    }
    Err(Exception::throw_message(
        ctx,
        "SftpClient.put takes text, a Uint8Array, an uploaded file, or a resource reference",
    ))
}

fn sftp_new<'js>(
    ctx: Ctx<'js>,
    host: String,
    username: String,
    port: Opt<JsValue<'js>>,
) -> rquickjs::Result<Class<'js, HostSftpClient>> {
    host_guard(&ctx, || {
        let port = match port.0.as_ref().and_then(JsValue::as_number) {
            Some(number) if (1.0..=65535.0).contains(&number) =>
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(number as u16)
            }
            _ => None,
        };
        let client = SftpClient::new(&host, &username, port).map_err(|err| throw_lib(&ctx, &err))?;
        Class::instance(
            ctx.clone(),
            HostSftpClient {
                inner: RefCell::new(client),
            },
        )
    })
}

/// Define the global `SftpClient`.
pub(super) fn install(ctx: &Ctx<'_>) -> Result<(), Error> {
    let make = caught(ctx, "SftpClient", Function::new(ctx.clone(), sftp_new))?;
    let wrap: Function = eval_js(ctx, SFTP_CLIENT_JS)?;
    let class: JsValue = caught(ctx, "SftpClient", wrap.call((make,)))?;
    caught(ctx, "globals", ctx.globals().set("SftpClient", class))
}

#[cfg(test)]
mod tests {
    use crate::script::eval_value;
    use crate::sftp::tests::memory;
    use moqui_lib::object::{to_plain_string, Value};

    #[test]
    fn a_script_uses_sftp_client_as_groovy_did() {
        let context = crate::context::ExecutionContextFactory::test_context();
        let _entered = context.enter();
        let connector = memory(context.factory());
        let script = r#"(function () {
            var out = [];
            var c = new SftpClient("files.example.com", "joe");
            try { c.lsFiles("/in"); } catch (e) { out.push(String(e.message)); }
            try { c.connect(); } catch (e) { out.push(String(e.message)); }
            c.password("pw").preserveAttributes(false).connect();
            c.put("/in", "a.txt", "café", "ISO-8859-1").put("/in/b.bin", "x", new Uint8Array([1, 2, 3]));
            c.put("/new/dir/c.txt", "c", "see", null, true);
            out.push(c.lsFiles("/in").join(","));
            var listed = c.ls("/in");
            out.push(listed[0].name + ":" + listed[0].path + ":" + listed[0].isFile + ":" + listed[0].size);
            out.push(c.getText("/in/a.txt", "ISO-8859-1"));
            out.push(c.getBinary("/in/b.bin").length);
            var w = c.putStream("/in/s.txt"); w.write("one "); w.write(new Uint8Array([116, 119, 111])); w.close();
            var r = c.openStream("/in/s.txt"); out.push(r.readText()); r.close();
            out.push(c.moveFile("/in/a.txt", "/in/done"));
            out.push(c.renameFile("/in/s.txt", "t.txt"));
            c.rm("/in/t.txt");
            try { c.publicKey(null, null); } catch (e) { out.push(String(e.message).includes("publicKeyPkcs8")); }
            c.close();
            try { c.getText("/in/b.bin"); } catch (e) { out.push(String(e.message)); }
            out.push(SftpClient.SFTP_PORT);
            return out;
        })()"#;
        let result = eval_value(script);
        let Value::List(items) = result.unwrap() else {
            panic!("not a list");
        };
        let texts: Vec<String> = items.iter().map(to_plain_string).collect();
        assert_eq!(
            texts,
            [
                "SFTP Client not connected",
                "No password or KeyProvider specified, cannot connect to joe@files.example.com:22",
                "a.txt,b.bin",
                "a.txt:/in/a.txt:true:4",
                "caf\u{e9}",
                "3",
                "one two",
                "/in/done/a.txt",
                "/in/t.txt",
                "true",
                "SFTP Client not connected",
                "115",
            ]
        );
        assert!(connector.disk.lock().unwrap().files.contains_key("/new/dir/c.txt"));
    }
}
