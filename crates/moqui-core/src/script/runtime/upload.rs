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

//! An uploaded file as a script sees it: what the Java `FileItem` showed (name, type, size) and the ways to
//! keep the bytes. A script cannot hold binary data, so the bytes go to a `dbresource://` location or are read
//! as text.

use std::sync::Arc;

use super::*;
use crate::web::multipart::UploadedFile;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostUpload {
    #[qjs(skip_trace)]
    pub(super) inner: Arc<UploadedFile>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostUpload {
    #[qjs(get, rename = "name")]
    fn name_property(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.file_name.clone()))
    }

    #[qjs(get, rename = "fileName")]
    fn file_name_property(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.file_name.clone()))
    }

    #[qjs(get, rename = "fieldName")]
    fn field_name_property(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.field_name.clone()))
    }

    #[qjs(get, rename = "contentType")]
    fn content_type_property(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.content_type.clone()))
    }

    #[qjs(get, rename = "size")]
    fn size_property<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            i64_to_js(&ctx, i64::try_from(self.inner.bytes.len()).unwrap_or(i64::MAX))
        })
    }

    fn get_name(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.file_name.clone()))
    }

    fn get_field_name(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.field_name.clone()))
    }

    fn get_content_type(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.content_type.clone()))
    }

    fn get_size<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            i64_to_js(&ctx, i64::try_from(self.inner.bytes.len()).unwrap_or(i64::MAX))
        })
    }

    fn is_in_memory(&self) -> bool {
        true
    }

    /// The bytes as text in `charset` (Java's `FileItem.getString(encoding)`), UTF-8 without one.
    fn get_string(&self, ctx: Ctx<'_>, charset: Opt<JsValue<'_>>) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            decode_text(&self.inner.bytes, charset_arg(&charset).as_deref())
                .map_err(|err| Exception::throw_message(&ctx, err.message()))
        })
    }

    /// The bytes as a stream, as the Java `FileItem.getInputStream`. A script hands it to `putStream` of a resource
    /// reference and closes it.
    fn get_input_stream<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostStream>> {
        host_guard(&ctx, || {
            Class::instance(ctx.clone(), HostStream::new(self.inner.bytes.clone()))
        })
    }

    /// Store the bytes at a `dbresource://` location.
    fn write(&self, ctx: Ctx<'_>, location: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            with_ec(&ctx, |ec| {
                ec.factory().resource().write_bytes(&location, &self.inner.bytes)
            })?
            .map_err(|err| Exception::throw_message(&ctx, err.message()))
        })
    }

    #[qjs(rename = "toString")]
    fn to_string_text(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.file_name.clone()))
    }
}

/// The bytes of an uploaded file or a resource, as the Java `InputStream` a script passes to `putStream`. A script
/// cannot read it a byte at a time: it reads all of it as text or a `Uint8Array`, or writes it to a location.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostStream {
    #[qjs(skip_trace)]
    bytes: Vec<u8>,
}

impl HostStream {
    pub(super) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostStream {
    /// Nothing is held open, so closing is only for the script's `try`/`finally`.
    fn close(&self) {}

    fn available<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            i64_to_js(&ctx, i64::try_from(self.bytes.len()).unwrap_or(i64::MAX))
        })
    }

    fn read_all_bytes<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            rquickjs::TypedArray::<u8>::new(ctx.clone(), self.bytes.clone()).map(rquickjs::TypedArray::into_value)
        })
    }

    /// The bytes as text in `charset`, UTF-8 without one.
    fn get_text(&self, ctx: Ctx<'_>, charset: Opt<JsValue<'_>>) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            decode_text(&self.bytes, charset_arg(&charset).as_deref())
                .map_err(|err| Exception::throw_message(&ctx, err.message()))
        })
    }

    /// Store the bytes at a location, as `putStream` of that location's reference does.
    fn write(&self, ctx: Ctx<'_>, location: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            with_ec(&ctx, |ec| ec.factory().resource().write_bytes(&location, &self.bytes))?
                .map_err(|err| Exception::throw_message(&ctx, err.message()))
        })
    }
}

/// A charset name given to a script call: a non-empty string, else none (a `cache` boolean of `getLocationText`).
pub(super) fn charset_arg(value: &Opt<JsValue<'_>>) -> Option<String> {
    let text = value.0.as_ref()?.as_string()?.to_string().ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Bytes as text in a Java charset name, as `new String(bytes, charset)`: UTF-8 when none is given (malformed sequences
/// become U+FFFD), ISO-8859-1 byte for byte, UTF-16 by its byte order mark (big-endian without one, as Java), and the other
/// names of the WHATWG Encoding Standard (windows-1252, UTF-16LE, Shift_JIS, ...). A byte order mark of the charset
/// itself is not removed, as Java keeps it. An unknown name is an error, as Java's `UnsupportedEncodingException`.
pub(crate) fn decode_text(bytes: &[u8], charset: Option<&str>) -> Result<String, crate::context::Error> {
    let Some(charset) = charset.map(str::trim).filter(|name| !name.is_empty()) else {
        return Ok(String::from_utf8_lossy(bytes).into_owned());
    };
    let upper = charset.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "ISO-8859-1" | "ISO8859-1" | "ISO8859_1" | "LATIN1" | "ISO-LATIN-1"
    ) {
        return Ok(bytes.iter().map(|byte| char::from(*byte)).collect());
    }
    if upper == "UTF-16" || upper == "UTF16" {
        let little = bytes.starts_with(&[0xFF, 0xFE]);
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| {
                let (first, second) = (pair[0], pair.get(1).copied().unwrap_or(0));
                if little {
                    u16::from_le_bytes([first, second])
                } else {
                    u16::from_be_bytes([first, second])
                }
            })
            .collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    let encoding = crate::sftp::encoding_for(charset)?;
    Ok(crate::sftp::decode(encoding, bytes))
}

/// An uploaded file made from text, as the JavaScript object a service or spec holds. The test harness uses it to call
/// the services that take a `FileItem` without a web request.
pub(crate) fn text_upload_to_js<'js>(
    ctx: &Ctx<'js>,
    field_name: &str,
    file_name: &str,
    content_type: &str,
    text: &str,
) -> rquickjs::Result<JsValue<'js>> {
    let inner = Arc::new(UploadedFile {
        field_name: field_name.to_string(),
        file_name: file_name.to_string(),
        content_type: content_type.to_string(),
        bytes: text.as_bytes().to_vec(),
    });
    Class::instance(ctx.clone(), HostUpload { inner }).and_then(|class| class.into_js(ctx))
}
