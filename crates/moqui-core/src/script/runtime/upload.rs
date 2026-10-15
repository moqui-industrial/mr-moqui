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

    /// The bytes as UTF-8 text.
    fn get_string(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(String::from_utf8_lossy(&self.inner.bytes).into_owned()))
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
