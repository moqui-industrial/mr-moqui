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

//! `ec.makeNotificationMessage()`: Java's `NotificationMessage` builder, backed by [`crate::notify::NotificationMessage`].
//! Each setter changes the builder and returns it.

use std::cell::RefCell;

use super::*;
use crate::notify::NotificationMessage;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostNotification {
    #[qjs(skip_trace)]
    inner: RefCell<NotificationMessage>,
}

pub(super) fn make_notification_message<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostNotification>> {
    host_guard(&ctx, || {
        Class::instance(
            ctx.clone(),
            HostNotification {
                inner: RefCell::new(NotificationMessage::new()),
            },
        )
    })
}

/// Change the builder `this` and return it.
fn changed<'js>(
    this: This<Class<'js, HostNotification>>,
    ctx: &Ctx<'js>,
    change: impl FnOnce(&mut NotificationMessage),
) -> rquickjs::Result<Class<'js, HostNotification>> {
    {
        let builder = this.0.borrow();
        let mut message = builder
            .inner
            .try_borrow_mut()
            .map_err(|_| Exception::throw_message(ctx, "the notification is being sent"))?;
        change(&mut message);
    }
    Ok(this.0)
}

fn text_of(ctx: &Ctx<'_>, value: &JsValue<'_>) -> rquickjs::Result<String> {
    Ok(moqui_lib::object::to_plain_string(&js_to_host(ctx, value)?))
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostNotification {
    fn topic<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        topic: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let topic = text_of(&ctx, &topic)?;
            changed(this, &ctx, |message| {
                message.topic(topic);
            })
        })
    }

    fn sub_topic<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        sub_topic: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let sub_topic = text_of(&ctx, &sub_topic)?;
            changed(this, &ctx, |message| {
                message.sub_topic(sub_topic);
            })
        })
    }

    fn user_id<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        user_id: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let user_id = text_of(&ctx, &user_id)?;
            changed(this, &ctx, |message| {
                message.user_id(user_id);
            })
        })
    }

    /// An array or a set of user IDs, added to those set.
    fn user_ids<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        user_ids: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let ids = host_texts(js_to_host(&ctx, &user_ids)?);
            changed(this, &ctx, |message| {
                message.user_ids(ids);
            })
        })
    }

    fn user_group_id<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        group_id: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let group_id = text_of(&ctx, &group_id)?;
            changed(this, &ctx, |message| {
                message.user_group_id(group_id);
            })
        })
    }

    /// `message(map)` or `message(jsonText)`.
    fn message<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        body: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || match js_to_host(&ctx, &body)? {
            Value::Map(map) => changed(this, &ctx, |message| {
                message.message_map(map);
            }),
            Value::Null => changed(this, &ctx, |message| {
                message.message_map(indexmap::IndexMap::new());
            }),
            other => {
                let json = moqui_lib::object::to_plain_string(&other);
                changed(this, &ctx, |message| {
                    message.message_json(json);
                })
            }
        })
    }

    fn title<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        title: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let title = text_of(&ctx, &title)?;
            changed(this, &ctx, |message| {
                message.title(title);
            })
        })
    }

    fn link<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        link: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let link = text_of(&ctx, &link)?;
            changed(this, &ctx, |message| {
                message.link(link);
            })
        })
    }

    /// `type(name)`: `info`, `success`, `warning`, or `danger`.
    #[qjs(rename = "type")]
    fn type_name<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        type_name: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let type_name = text_of(&ctx, &type_name)?;
            changed(this, &ctx, |message| {
                message.notification_type(type_name);
            })
        })
    }

    fn show_alert<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let flag = flag.as_bool().unwrap_or(true);
            changed(this, &ctx, |message| {
                message.show_alert(flag);
            })
        })
    }

    fn alert_no_auto_hide<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let flag = flag.as_bool().unwrap_or(true);
            changed(this, &ctx, |message| {
                message.alert_no_auto_hide(flag);
            })
        })
    }

    fn persist_on_send<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let flag = flag.as_bool().unwrap_or(true);
            changed(this, &ctx, |message| {
                message.persist(flag);
            })
        })
    }

    fn email_template_id<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        id: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let id = text_of(&ctx, &id)?;
            changed(this, &ctx, |message| {
                message.email_template_id(id);
            })
        })
    }

    fn email_message_save<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            let flag = flag.as_bool().unwrap_or(true);
            changed(this, &ctx, |message| {
                message.email_message_save(flag);
            })
        })
    }

    /// `send()` or `send(persist)`: returns the notification, so `getNotificationMessageId()` follows.
    fn send<'js>(
        this: This<Class<'js, HostNotification>>,
        ctx: Ctx<'js>,
        persist: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<Class<'js, HostNotification>> {
        host_guard(&ctx, || {
            {
                let builder = this.0.borrow();
                let mut message = builder
                    .inner
                    .try_borrow_mut()
                    .map_err(|_| Exception::throw_message(&ctx, "the notification is being sent"))?;
                if let Some(flag) = persist.0.as_ref().and_then(JsValue::as_bool) {
                    message.persist(flag);
                }
                message
                    .send()
                    .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
            }
            Ok(this.0)
        })
    }

    fn get_topic(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.inner.borrow().topic_name().to_string()))
    }

    /// The `NotificationTopic` value of the topic, or null when the topic has none.
    fn get_notification_topic<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let found = self
                .inner
                .borrow()
                .notification_topic()
                .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
            match found {
                Some(value) => entity_to_js(&ctx, HostEntity::new(value)),
                None => Null.into_js(&ctx),
            }
        })
    }

    /// The title as expanded by the last send, null before.
    fn get_title<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || optional_string(&ctx, self.inner.borrow().title_text()))
    }

    fn get_link<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || optional_string(&ctx, self.inner.borrow().link_text()))
    }

    fn get_notification_message_id<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            optional_string(&ctx, self.inner.borrow().notification_message_id())
        })
    }

    fn get_notify_user_ids<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let ids: Vec<Value> = self
                .inner
                .borrow()
                .notify_user_ids()
                .iter()
                .map(Value::string)
                .collect();
            host_to_js(&ctx, Value::List(ids))
        })
    }
}

fn optional_string<'js>(ctx: &Ctx<'js>, text: Option<&str>) -> rquickjs::Result<JsValue<'js>> {
    match text {
        Some(text) => host_to_js(ctx, Value::string(text)),
        None => Null.into_js(ctx),
    }
}
