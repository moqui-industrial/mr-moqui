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

//! Entity finds, lists, and values as scripts see them.

use super::*;

pub(super) fn entity_find<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostFind>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let find = facade.find(&name).map_err(|err| throw_lib(&ctx, &err))?;
        wrap_find(&ctx, find)
    })
}

pub(super) fn entity_make_value<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostEntity>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let value = facade.make_value(&name).map_err(|err| throw_lib(&ctx, &err))?;
        Class::instance(ctx.clone(), HostEntity::new(value))
    })
}

pub(super) fn wrap_find<'js>(ctx: &Ctx<'js>, find: EntityFind) -> rquickjs::Result<Class<'js, HostFind>> {
    Class::instance(
        ctx.clone(),
        HostFind {
            inner: RefCell::new(Some(find)),
        },
    )
}

pub(super) fn take_find(ctx: &Ctx<'_>, slot: &RefCell<Option<EntityFind>>) -> rquickjs::Result<EntityFind> {
    slot.borrow_mut()
        .take()
        .ok_or_else(|| Exception::throw_message(ctx, "entity find was already used"))
}

pub(super) fn prefixed_field(prefix: &str, field: &str) -> String {
    if prefix.is_empty() {
        return field.to_string();
    }
    let mut name = String::with_capacity(prefix.len() + field.len());
    name.push_str(prefix);
    let mut chars = field.chars();
    if let Some(first) = chars.next() {
        name.push(first.to_ascii_uppercase());
        name.push_str(chars.as_str());
    }
    name
}

pub(super) enum FieldInclude {
    All,
    Pk,
    NonPk,
}

pub(super) fn field_include(value: &JsValue<'_>) -> FieldInclude {
    if value.is_null() || value.is_undefined() {
        return FieldInclude::All;
    }
    match value.as_bool() {
        Some(true) => FieldInclude::Pk,
        Some(false) => FieldInclude::NonPk,
        None => FieldInclude::All,
    }
}

pub(super) fn field_prefix(ctx: &Ctx<'_>, value: &JsValue<'_>) -> rquickjs::Result<String> {
    if value.is_null() || value.is_undefined() {
        return Ok(String::new());
    }
    js_string(ctx, value)
}

/// A find builder. Chain methods take the inner find so a second use fails.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostFind {
    #[qjs(skip_trace)]
    pub(super) inner: RefCell<Option<EntityFind>>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostFind {
    fn condition<'js>(
        &self,
        ctx: Ctx<'js>,
        first: JsValue<'js>,
        second: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let mut find = take_find(&ctx, &self.inner)?;
            if second.0.is_none() {
                if first.is_null() || first.is_undefined() {
                    return wrap_find(&ctx, find);
                }
                if let Some(condition) = condition_of(&first) {
                    return wrap_find(&ctx, find.condition(condition));
                }
            }
            if let Some(value) = second.0 {
                let field = js_string(&ctx, &first)?;
                let host = js_to_host(&ctx, &value)?;
                let condition = comparison(&field, "equals", host).map_err(|err| throw_lib(&ctx, &err))?;
                return wrap_find(&ctx, find.condition(condition));
            }
            let Some(object) = first.as_object() else {
                return Err(Exception::throw_message(
                    &ctx,
                    "entity condition needs a field and a value",
                ));
            };
            // A map such as `context` carries names that are not fields. Only fields apply.
            let facade = facade_of(&ctx)?;
            let known: std::collections::BTreeSet<String> = facade
                .with_entity(find.entity_name(), |entity, _, _| {
                    Ok(entity.fields.iter().map(|field| field.name.clone()).collect())
                })
                .map_err(|err| throw_lib(&ctx, &err))?;
            for key in object.keys::<String>() {
                let key = key?;
                if !known.contains(&key) {
                    continue;
                }
                let property: JsValue = object.get(key.as_str())?;
                let host = js_to_host(&ctx, &property)?;
                let condition = comparison(&key, "equals", host).map_err(|err| throw_lib(&ctx, &err))?;
                find = find.condition(condition);
            }
            wrap_find(&ctx, find)
        })
    }

    fn order_by<'js>(&self, ctx: Ctx<'js>, field: String) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let (name, descending) = match field.strip_prefix('-') {
                Some(rest) => (rest.to_string(), true),
                None => (field, false),
            };
            let find = take_find(&ctx, &self.inner)?.order_by(&name, descending);
            wrap_find(&ctx, find)
        })
    }

    fn limit<'js>(&self, ctx: Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let number = js_i64(&ctx, &value)?;
            let find = take_find(&ctx, &self.inner)?.limit(number);
            wrap_find(&ctx, find)
        })
    }

    fn offset<'js>(&self, ctx: Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let number = js_i64(&ctx, &value)?;
            let find = take_find(&ctx, &self.inner)?.offset(number);
            wrap_find(&ctx, find)
        })
    }

    /// Reads go to the entity cache or the database as `EntityFind` decides, so the cache and
    /// clone hints change nothing.
    fn use_cache<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
    }

    fn for_update<'js>(&self, ctx: Ctx<'js>, flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let find = take_find(&ctx, &self.inner)?.for_update(flag.as_bool().unwrap_or(true));
            wrap_find(&ctx, find)
        })
    }

    fn distinct<'js>(&self, ctx: Ctx<'js>, flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let find = take_find(&ctx, &self.inner)?.distinct(flag.as_bool().unwrap_or(true));
            wrap_find(&ctx, find)
        })
    }

    fn use_clone<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
    }

    fn select_field<'js>(&self, ctx: Ctx<'js>, field: String) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let find = take_find(&ctx, &self.inner)?.select_field(&field);
            wrap_find(&ctx, find)
        })
    }

    fn list<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            let list = take_find(&ctx, &self.inner)?
                .list()
                .map_err(|err| throw_lib(&ctx, &err))?;
            Class::instance(ctx.clone(), HostList { inner: Arc::new(list) })
        })
    }

    fn one<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let found = take_find(&ctx, &self.inner)?
                .one()
                .map_err(|err| throw_lib(&ctx, &err))?;
            match found {
                Some(value) => {
                    let class = Class::instance(ctx.clone(), HostEntity::new(value))?;
                    class.into_js(&ctx)
                }
                None => Null.into_js(&ctx),
            }
        })
    }

    fn count<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let number = take_find(&ctx, &self.inner)?
                .count()
                .map_err(|err| throw_lib(&ctx, &err))?;
            i64_to_js(&ctx, number)
        })
    }

    fn delete_all(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            let list = take_find(&ctx, &self.inner)?
                .list()
                .map_err(|err| throw_lib(&ctx, &err))?;
            for value in list.values() {
                value.delete(&facade).map_err(|err| throw_lib(&ctx, &err))?;
            }
            Ok(())
        })
    }

    /// A script reads the whole result. A streaming cursor is the host's `EntityFind::iterator`.
    fn iterator<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostList>> {
        self.list(ctx)
    }

    /// A condition on an aggregate alias of a view-entity. A null condition is skipped.
    fn having_condition<'js>(&self, ctx: Ctx<'js>, condition: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let mut find = take_find(&ctx, &self.inner)?;
            if let Some(condition) = condition_of(&condition) {
                find = find.having(condition);
            }
            wrap_find(&ctx, find)
        })
    }

    /// `searchFormMap(inputFields, defaultParameters, skipFields, defaultOrderBy, paginate)`.
    /// The defaults and `orderByField` it applied are put in the context, so the form shows them.
    fn search_form_map<'js>(&self, ctx: Ctx<'js>, args: Rest<JsValue<'js>>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let map_arg = |index: usize| -> rquickjs::Result<std::collections::BTreeMap<String, Value>> {
                match args.0.get(index) {
                    Some(value) if !value.is_null() && !value.is_undefined() => match js_to_host(&ctx, value)? {
                        Value::Map(entries) => Ok(entries.into_iter().collect()),
                        _ => Err(Exception::throw_message(&ctx, "searchFormMap needs maps of fields")),
                    },
                    _ => Ok(std::collections::BTreeMap::new()),
                }
            };
            let text_arg = |index: usize| -> String {
                args.0
                    .get(index)
                    .and_then(|value| js_to_host(&ctx, value).ok())
                    .map(|value| moqui_lib::object::to_plain_string(&value))
                    .unwrap_or_default()
            };
            let input = map_arg(0)?;
            let defaults = map_arg(1)?;
            let paginate = args.0.get(4).and_then(JsValue::as_bool).unwrap_or(false);
            let found = take_find(&ctx, &self.inner)?
                .search_form_map(&input, &defaults, &text_arg(2), &text_arg(3), paginate)
                .map_err(|err| throw_lib(&ctx, &err))?;
            for (name, value) in found.context {
                put_name(&ctx, &name, value)?;
            }
            wrap_find(&ctx, found.find)
        })
    }

    fn require_search_form_parameters<'js>(
        &self,
        ctx: Ctx<'js>,
        require: Opt<bool>,
    ) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let find = take_find(&ctx, &self.inner)?.require_search_form_parameters(require.0.unwrap_or(true));
            wrap_find(&ctx, find)
        })
    }
}

/// A find result. `length` and `get` are what iterate and inline scripts use.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostList {
    #[qjs(skip_trace)]
    pub(super) inner: Arc<EntityList>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostList {
    #[qjs(get)]
    fn length(&self, ctx: Ctx<'_>) -> rquickjs::Result<i32> {
        host_guard(&ctx, || {
            i32::try_from(self.inner.len()).map_err(|_| Exception::throw_message(&ctx, "list is too long"))
        })
    }

    fn get<'js>(&self, ctx: Ctx<'js>, index: i32) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            if index < 0 {
                return Null.into_js(&ctx);
            }
            let index =
                usize::try_from(index).map_err(|_| Exception::throw_message(&ctx, "list index is too large"))?;
            match self.inner.get(index) {
                Some(value) => {
                    let class = Class::instance(ctx.clone(), HostEntity::new(value.clone()))?;
                    class.into_js(&ctx)
                }
                None => Null.into_js(&ctx),
            }
        })
    }

    fn get_partial_list<'js>(
        &self,
        ctx: Ctx<'js>,
        offset: i32,
        size: i32,
        _count: Opt<bool>,
    ) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            let start = usize::try_from(offset.max(0)).unwrap_or(0);
            let take = usize::try_from(size.max(0)).unwrap_or(0);
            let values = self.inner.values().iter().skip(start).take(take).cloned().collect();
            Class::instance(
                ctx.clone(),
                HostList {
                    inner: Arc::new(EntityList::new(values)),
                },
            )
        })
    }

    fn filter_by_date<'js>(
        &self,
        ctx: Ctx<'js>,
        from_name: String,
        thru_name: String,
        valid: JsValue<'js>,
        ignore_if_empty: Opt<bool>,
    ) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            let valid = js_to_host(&ctx, &valid)?;
            let empty = moqui_lib::object::is_empty(&valid);
            let values = if empty && ignore_if_empty.0.unwrap_or(false) {
                self.inner.values().to_vec()
            } else {
                let valid = if empty { now_timestamp() } else { timestamp_of(&valid) };
                self.inner
                    .values()
                    .iter()
                    .filter(|row| {
                        let from = timestamp_of(row.get(&from_name).unwrap_or(&Value::Null));
                        let thru = timestamp_of(row.get(&thru_name).unwrap_or(&Value::Null));
                        let started = from.is_null()
                            || moqui_lib::object::compare_obj(&from, &valid)
                                .is_ok_and(|ordering| ordering != Ordering::Greater);
                        let open = thru.is_null()
                            || moqui_lib::object::compare_obj(&thru, &valid)
                                .is_ok_and(|ordering| ordering == Ordering::Greater);
                        started && open
                    })
                    .cloned()
                    .collect()
            };
            Class::instance(
                ctx.clone(),
                HostList {
                    inner: Arc::new(EntityList::new(values)),
                },
            )
        })
    }
}

/// One entity value. Field storage stays on this object, not on the context stack.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostEntity {
    #[qjs(skip_trace)]
    pub(super) inner: Arc<EntityCell>,
}

/// An entity value a script and the context stack share by reference.
pub(super) type EntityCell = Mutex<EntityValue>;

impl HostEntity {
    fn new(value: EntityValue) -> Self {
        Self {
            inner: Arc::new(Mutex::new(value)),
        }
    }

    fn val(&self) -> MutexGuard<'_, EntityValue> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostEntity {
    fn get<'js>(&self, ctx: Ctx<'js>, field: String) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || match self.val().get(&field) {
            Some(value) => value_to_js(&ctx, value),
            None => Null.into_js(&ctx),
        })
    }

    fn set(&self, ctx: Ctx<'_>, field: String, value: JsValue<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let host = js_to_host(&ctx, &value)?;
            self.val().set(field, host);
            Ok(())
        })
    }

    fn set_fields(
        &self,
        ctx: Ctx<'_>,
        map: JsValue<'_>,
        set_if_empty: Opt<bool>,
        prefix: JsValue<'_>,
        include: JsValue<'_>,
    ) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let set_empty = set_if_empty.0.unwrap_or(false);
            let prefix = field_prefix(&ctx, &prefix)?;
            let include = field_include(&include);
            let Some(object) = map.as_object() else {
                return Err(Exception::throw_message(&ctx, "setFields needs an object"));
            };
            let facade = facade_of(&ctx)?;
            let entity_name = self.val().entity_name().to_string();
            let specs = facade
                .with_entity(&entity_name, |entity, _, _| {
                    Ok(entity
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), field.is_pk))
                        .collect::<Vec<_>>())
                })
                .map_err(|err| throw_lib(&ctx, &err))?;
            for (name, is_pk) in specs {
                let keep = match include {
                    FieldInclude::All => true,
                    FieldInclude::Pk => is_pk,
                    FieldInclude::NonPk => !is_pk,
                };
                if !keep {
                    continue;
                }
                let key = prefixed_field(&prefix, &name);
                if !object.contains_key(key.as_str())? {
                    continue;
                }
                let property: JsValue = object.get(key.as_str())?;
                let host = js_to_host(&ctx, &property)?;
                let empty = host.is_null() || matches!(&host, Value::String(text) if text.is_empty());
                if !set_empty && empty {
                    continue;
                }
                self.val().set(name, host);
            }
            Ok(())
        })
    }

    fn create(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val().create(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn update(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val().update(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn delete(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val().delete(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn create_or_update(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val()
                .create_or_update(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn set_sequenced_id_primary(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val()
                .set_sequenced_id_primary(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn set_sequenced_id_secondary(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val()
                .set_sequenced_id_secondary(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn find_related<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        map: Opt<JsValue<'js>>,
        order: Opt<JsValue<'js>>,
        _cache: Opt<JsValue<'js>>,
        for_update: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            let lock = for_update.0.as_ref().and_then(JsValue::as_bool) == Some(true);
            let mut find = self.related_find(&ctx, &name)?.for_update(lock);
            if let Some(Value::Map(entries)) = map.0.as_ref().map(|value| js_to_host(&ctx, value)).transpose()? {
                for (key, value) in entries {
                    find = find.eq(&key, value);
                }
            }
            if let Some(Value::List(fields)) = order.0.as_ref().map(|value| js_to_host(&ctx, value)).transpose()? {
                for field in fields {
                    let text = moqui_lib::object::to_plain_string(&field);
                    match text.strip_prefix('-') {
                        Some(rest) => find = find.order_by(rest, true),
                        None => find = find.order_by(text.trim_start_matches('+'), false),
                    }
                }
            }
            let list = find.list().map_err(|err| throw_lib(&ctx, &err))?;
            Class::instance(ctx.clone(), HostList { inner: Arc::new(list) })
        })
    }

    fn find_related_one<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        _cache: Opt<JsValue<'js>>,
        for_update: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let lock = for_update.0.as_ref().and_then(JsValue::as_bool) == Some(true);
            let found = self
                .related_find(&ctx, &name)?
                .for_update(lock)
                .one()
                .map_err(|err| throw_lib(&ctx, &err))?;
            match found {
                Some(value) => {
                    Class::instance(ctx.clone(), HostEntity::new(value)).and_then(|class| class.into_js(&ctx))
                }
                None => Null.into_js(&ctx),
            }
        })
    }

    fn delete_related(&self, ctx: Ctx<'_>, name: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            let list = self
                .related_find(&ctx, &name)?
                .list()
                .map_err(|err| throw_lib(&ctx, &err))?;
            for value in list.values() {
                value.delete(&facade).map_err(|err| throw_lib(&ctx, &err))?;
            }
            Ok(())
        })
    }
}

impl HostEntity {
    fn related_find(&self, ctx: &Ctx<'_>, relationship: &str) -> rquickjs::Result<EntityFind> {
        let facade = facade_of(ctx)?;
        let record = self.val().clone();
        let (entity, conditions) =
            crate::service::call::related_conditions(&facade, record.entity_name(), relationship, &record)
                .map_err(|err| throw_lib(ctx, &err))?;
        let mut find = facade.find(&entity).map_err(|err| throw_lib(ctx, &err))?;
        for (field, value) in conditions {
            find = find.eq(&field, value);
        }
        Ok(find)
    }
}
