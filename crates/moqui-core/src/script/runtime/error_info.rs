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

//! `errorThrowable` for the html error pages, with the calls `error/*.xml` makes on it.

use std::sync::Arc;

use super::*;
use crate::web::error_info::ErrorInfo;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostErrorInfo {
    #[qjs(skip_trace)]
    pub(super) inner: Arc<ErrorInfo>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostErrorInfo {
    #[qjs(get, rename = "message")]
    fn message_property(&self) -> String {
        self.inner.message.clone()
    }

    fn get_message(&self) -> String {
        self.inner.message.clone()
    }

    #[qjs(get, rename = "pathFromLastScreen")]
    fn path_from_last_screen_property(&self) -> String {
        self.inner.path_from_last_screen.clone()
    }

    #[qjs(get, rename = "fullPathNameList")]
    fn full_path_name_list_property(&self) -> Vec<String> {
        self.inner.full_path.clone()
    }

    #[qjs(get, rename = "resourceLocation")]
    fn resource_location_property(&self) -> String {
        String::new()
    }

    #[qjs(get, rename = "lastSd")]
    fn last_sd_property<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        Null.into_js(&ctx)
    }

    /// The denied artifact as `{name, actionDescription, typeDescription}` with the Java accessors, or null.
    fn get_artifact_info<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        let Some(info) = self.inner.artifact.clone() else {
            return Null.into_js(&ctx);
        };
        let object = Object::new(ctx.clone())?;
        let (name, action, kind) = (info.name.clone(), info.action_description(), info.type_description());
        let text = format!("{kind} {name} ({action})");
        object.set("name", name.clone())?;
        object.set("actionDescription", action.clone())?;
        object.set("typeDescription", kind.clone())?;
        object.set("getName", Function::new(ctx.clone(), move || name.clone())?)?;
        object.set(
            "getActionDescription",
            Function::new(ctx.clone(), move || action.clone())?,
        )?;
        object.set("getTypeDescription", Function::new(ctx.clone(), move || kind.clone())?)?;
        object.set("toString", Function::new(ctx.clone(), move || text.clone())?)?;
        Ok(object.into_value())
    }

    fn get_artifact_stack(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_stack_trace(&self) -> Vec<String> {
        self.inner.trace.clone()
    }
}
