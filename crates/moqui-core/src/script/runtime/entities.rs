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

use rquickjs::class::{PropertyDescriptor, PropertyName};
use rquickjs::Atom;

use super::*;

pub(super) fn entity_find<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostFind>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let find = facade.find(&name).map_err(|err| throw_lib(&ctx, &err))?;
        wrap_find(&ctx, find)
    })
}

pub(super) fn entity_make_value<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let value = facade.make_value(&name).map_err(|err| throw_lib(&ctx, &err))?;
        entity_to_js(&ctx, HostEntity::new(value))
    })
}

/// An entity value as a script sees it.
pub(super) fn entity_to_js<'js>(ctx: &Ctx<'js>, host: HostEntity) -> rquickjs::Result<JsValue<'js>> {
    Class::instance(ctx.clone(), host).and_then(|class| class.into_js(ctx))
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

impl HostFind {
    /// A clone of the find this object holds, which stays usable.
    fn peek(&self, ctx: &Ctx<'_>) -> rquickjs::Result<EntityFind> {
        self.inner
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| Exception::throw_message(ctx, "this find was already run"))
    }
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

    /// Comma separated fields, each with an optional `-` for descending, `+`, or `^` for ignore case.
    fn order_by<'js>(&self, ctx: Ctx<'js>, field: String) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let mut find = take_find(&ctx, &self.inner)?;
            for part in field.split(',') {
                find = find.order_by_text(part);
            }
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

    /// `useCache(flag)`: through the entity cache for any entity not marked `never`, as Java's `EntityFind.useCache`. A
    /// null or undefined flag leaves the entity's own setting.
    fn use_cache<'js>(&self, ctx: Ctx<'js>, flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let find = take_find(&ctx, &self.inner)?;
            let find = if flag.is_null() || flag.is_undefined() {
                find
            } else {
                let on = flag.as_bool().unwrap_or_else(|| {
                    flag.as_string()
                        .and_then(|text| text.to_string().ok())
                        .is_some_and(|text| text == "true")
                });
                find.use_cache(on)
            };
            wrap_find(&ctx, find)
        })
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

    fn disable_authz<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?.disable_authz()))
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

    /// The shown and hidden fields of a form-list, for its `entity-find` (generated code only).
    fn select_form_fields<'js>(&self, ctx: Ctx<'js>, fields: String) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let find = take_find(&ctx, &self.inner)?.select_form_fields(&fields);
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
                Some(value) => entity_to_js(&ctx, HostEntity::new(value)),
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

    /// `getLimit()`: the limit of the find, or null. Does not use the find up.
    fn get_limit<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || match self.peek(&ctx)?.limit_value() {
            Some(limit) => i64_to_js(&ctx, limit),
            None => Null.into_js(&ctx),
        })
    }

    /// `getOffset()`: the offset of the find, or null. Does not use the find up.
    fn get_offset<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || match self.peek(&ctx)?.offset_value() {
            Some(offset) => i64_to_js(&ctx, offset),
            None => Null.into_js(&ctx),
        })
    }

    /// A copy of the find, so a page's `list()` and the `count()` of all rows can both run.
    fn copy<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let find = self.peek(&ctx)?;
            wrap_find(&ctx, find)
        })
    }

    /// Delete the rows the find matches. Returns how many, as Java's `deleteAll`.
    fn delete_all<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            let list = take_find(&ctx, &self.inner)?
                .list()
                .map_err(|err| throw_lib(&ctx, &err))?;
            for value in list.values() {
                value.delete(&facade).map_err(|err| throw_lib(&ctx, &err))?;
            }
            i64_to_js(&ctx, i64::try_from(list.len()).unwrap_or(i64::MAX))
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

/// The exotic methods `#[rquickjs::class(exotic)]` calls, in the module it names `__impl_exotic_<Class>__`, written
/// here rather than by `#[rquickjs::exotic]` because that macro drops the receiver of a set. A set whose receiver is
/// another object, as `Object.create(value).x = 1` reaches the value through the prototype chain, defines the
/// property on the receiver, as an ordinary object's set does, rather than writing the field. Re-check when rquickjs
/// is upgraded.
macro_rules! exotic_module {
    ($module:ident, $class:ident) => {
        #[allow(non_snake_case)]
        mod $module {
            use super::*;
            use rquickjs::class::JsCell;

            pub(crate) struct ExoticImpl;

            impl ExoticImpl {
                pub fn exotic_get_property<'js>(
                    this: &JsCell<'js, $class>,
                    ctx: &Ctx<'js>,
                    atom: Atom<'js>,
                    _receiver: JsValue<'js>,
                ) -> rquickjs::Result<JsValue<'js>> {
                    this.borrow().property(ctx, atom)
                }

                pub fn exotic_set_property<'js>(
                    this: &JsCell<'js, $class>,
                    ctx: &Ctx<'js>,
                    atom: Atom<'js>,
                    receiver: JsValue<'js>,
                    value: JsValue<'js>,
                ) -> rquickjs::Result<bool> {
                    if is_receiver(this, &receiver) {
                        this.borrow().assign(ctx, atom, value)
                    } else {
                        define_on_receiver(atom, &receiver, value)
                    }
                }

                pub fn exotic_delete_property<'js>(
                    this: &JsCell<'js, $class>,
                    _ctx: &Ctx<'js>,
                    atom: Atom<'js>,
                ) -> rquickjs::Result<bool> {
                    Ok(this.borrow().remove(atom))
                }

                pub fn exotic_has_property<'js>(
                    this: &JsCell<'js, $class>,
                    ctx: &Ctx<'js>,
                    atom: Atom<'js>,
                ) -> rquickjs::Result<bool> {
                    this.borrow().has_member(ctx, atom)
                }

                pub fn exotic_get_own_property<'js>(
                    this: &JsCell<'js, $class>,
                    ctx: &Ctx<'js>,
                    atom: Atom<'js>,
                ) -> rquickjs::Result<Option<PropertyDescriptor<'js>>> {
                    this.borrow().own_property(ctx, atom)
                }

                pub fn exotic_get_own_property_names<'js>(
                    this: &JsCell<'js, $class>,
                    ctx: &Ctx<'js>,
                ) -> rquickjs::Result<Vec<PropertyName<'js>>> {
                    this.borrow().own_names(ctx)
                }
            }
        }
    };
}

/// True when a set's receiver is the exotic object itself, not an object that has it on its prototype chain.
fn is_receiver<'js, C: rquickjs::class::JsClass<'js>>(
    this: &rquickjs::class::JsCell<'js, C>,
    receiver: &JsValue<'js>,
) -> bool {
    receiver
        .as_object()
        .and_then(Class::<C>::from_object)
        .is_some_and(|class| std::ptr::eq(class.get_cell(), this))
}

/// The ordinary set of a property that is not on the receiver: a new own data property of the receiver. A receiver
/// that is not an object takes no property.
fn define_on_receiver<'js>(atom: Atom<'js>, receiver: &JsValue<'js>, value: JsValue<'js>) -> rquickjs::Result<bool> {
    let result = match receiver.as_object() {
        Some(object) => object
            .prop(
                atom,
                rquickjs::object::Property::from(value.clone())
                    .writable()
                    .enumerable()
                    .configurable(),
            )
            .map(|()| true),
        None => Ok(false),
    };
    keep_assigned(value);
    result
}

/// A find result. `length`, an index and `get` are what iterate and inline scripts use, and it is iterable as a
/// Java `List` is to Groovy. The methods written in JavaScript are added by [`install_entity_prototypes`].
#[derive(Trace, JsLifetime)]
#[rquickjs::class(exotic)]
pub(super) struct HostList {
    #[qjs(skip_trace)]
    pub(super) inner: Arc<EntityList>,
}

impl HostList {
    fn length(&self, ctx: &Ctx<'_>) -> rquickjs::Result<i32> {
        i32::try_from(self.inner.len()).map_err(|_| Exception::throw_message(ctx, "list is too long"))
    }

    /// The position a property name stands for, when it is an index in the list.
    fn index_of(&self, name: &str) -> Option<usize> {
        let index: usize = name.parse().ok()?;
        (index < self.inner.len() && index.to_string() == name).then_some(index)
    }

    fn item<'js>(&self, ctx: &Ctx<'js>, index: usize) -> rquickjs::Result<JsValue<'js>> {
        match self.inner.get(index) {
            Some(value) => {
                entity_to_js(ctx, HostEntity::new(value.clone()))
            }
            None => Null.into_js(ctx),
        }
    }
}

exotic_module!(__impl_exotic_HostList__, HostList);

/// Property access on a list: a prototype method first, then `length` and the index of a value.
impl HostList {
    fn property<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(ctx, || {
            if let Some(found) = prototype_member::<HostList>(ctx, &atom)? {
                return Ok(found);
            }
            let Some(name) = atom_name(&atom)? else {
                return Undefined.into_js(ctx);
            };
            if name == "length" {
                return self.length(ctx)?.into_js(ctx);
            }
            match self.index_of(&name) {
                Some(index) => self.item(ctx, index),
                None => Undefined.into_js(ctx),
            }
        })
    }

    /// A list is read-only: an assignment is ignored, as on a frozen array.
    fn assign<'js>(&self, _ctx: &Ctx<'js>, _atom: Atom<'js>, value: JsValue<'js>) -> Result<bool, JsError> {
        keep_assigned(value);
        Ok(false)
    }

    fn remove<'js>(&self, _atom: Atom<'js>) -> bool {
        false
    }

    fn has_member<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> Result<bool, JsError> {
        if prototype_has::<HostList>(ctx, &atom)? {
            return Ok(true);
        }
        Ok(atom_name(&atom)?.is_some_and(|name| name == "length" || self.index_of(&name).is_some()))
    }

    fn own_property<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> Result<Option<PropertyDescriptor<'js>>, JsError> {
        let Some(name) = atom_name(&atom)? else {
            return Ok(None);
        };
        if name == "length" {
            let length = self.length(ctx)?.into_js(ctx)?;
            return Ok(Some(PropertyDescriptor::new_value(length, false, false, false)));
        }
        match self.index_of(&name) {
            Some(index) => Ok(Some(PropertyDescriptor::new_value(
                self.item(ctx, index)?,
                true,
                true,
                false,
            ))),
            None => Ok(None),
        }
    }

    fn own_names<'js>(&self, ctx: &Ctx<'js>) -> Result<Vec<PropertyName<'js>>, JsError> {
        (0..self.inner.len())
            .map(|index| {
                let index = u32::try_from(index).map_err(|_| Exception::throw_message(ctx, "list is too long"))?;
                Ok(PropertyName {
                    atom: Atom::from_u32(ctx.clone(), index)?,
                    is_enumerable: true,
                })
            })
            .collect()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostList {
    fn get<'js>(&self, ctx: Ctx<'js>, index: i32) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            if index < 0 {
                return Null.into_js(&ctx);
            }
            let index =
                usize::try_from(index).map_err(|_| Exception::throw_message(&ctx, "list index is too large"))?;
            self.item(&ctx, index)
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
///
/// Fields read and write as properties too, as Groovy's map property syntax does over Java `EntityValueBase`.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(exotic)]
pub(super) struct HostEntity {
    #[qjs(skip_trace)]
    pub(super) inner: Arc<EntityCell>,
}

/// An entity value a script and the context stack share by reference.
pub(super) type EntityCell = Mutex<EntityValue>;

impl HostEntity {
    fn new(value: EntityValue) -> Self {
        Self::of(Arc::new(Mutex::new(value)))
    }

    pub(super) fn of(inner: Arc<EntityCell>) -> Self {
        Self { inner }
    }

    fn val(&self) -> MutexGuard<'_, EntityValue> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// What `name` is on the entity of this value, or None when no entity definition can be reached.
    fn member(&self, name: &str) -> Option<Member> {
        let facade = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten()?;
        let value = self.val();
        facade
            .with_entity(value.entity_name(), |entity, _, _| {
                if entity.has_field(name) {
                    return Ok(Member::Field);
                }
                Ok(match entity.relationship(name) {
                    Some(relationship) if relationship.relationship_type.starts_with("one") => Member::One,
                    Some(_) => Member::Many,
                    None => Member::Unknown,
                })
            })
            .ok()
    }

    /// Whether `name` is a field: one the value holds, or one its entity defines.
    fn is_field(&self, name: &str) -> bool {
        self.val().fields().contains_key(name) || matches!(self.member(name), Some(Member::Field))
    }

    /// Set a field, as `put` in Java: a name that is not a field of the entity is an error. When no entity
    /// definition can be reached, any name is set.
    fn set_field(&self, ctx: &Ctx<'_>, field: String, value: &JsValue<'_>) -> rquickjs::Result<()> {
        let host = js_to_host(ctx, value)?;
        let known = self.val().fields().contains_key(&field);
        if !known
            && self
                .member(&field)
                .is_some_and(|member| !matches!(member, Member::Field))
        {
            let entity_name = self.val().entity_name().to_string();
            return Err(Exception::throw_message(
                ctx,
                &format!("The field name {field} is not valid for entity {entity_name}"),
            ));
        }
        self.val().set(field, host);
        Ok(())
    }

    /// A field value, null for a field the value does not hold, or the related value or list for a relationship
    /// name, as Java `EntityValueBase.get` resolves a name. Undefined for any other name.
    fn read_member<'js>(&self, ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<JsValue<'js>> {
        let held = self.val().get(name).cloned();
        if let Some(value) = held {
            return host_to_js(ctx, value);
        }
        match self.member(name) {
            Some(Member::Field) => Null.into_js(ctx),
            Some(Member::One) => match self
                .related_find(ctx, name)?
                .one()
                .map_err(|err| throw_lib(ctx, &err))?
            {
                Some(value) => {
                    entity_to_js(ctx, HostEntity::new(value))
                }
                None => Null.into_js(ctx),
            },
            Some(Member::Many) => {
                let list = self
                    .related_find(ctx, name)?
                    .list()
                    .map_err(|err| throw_lib(ctx, &err))?;
                Class::instance(ctx.clone(), HostList { inner: Arc::new(list) }).and_then(|class| class.into_js(ctx))
            }
            Some(Member::Unknown) | None => Undefined.into_js(ctx),
        }
    }
}

exotic_module!(__impl_exotic_HostEntity__, HostEntity);

/// What a name that is not a method stands for on an entity value.
enum Member {
    Field,
    One,
    Many,
    Unknown,
}

/// Property access on an entity value. A prototype method wins over a field of the same name; a field name reads and
/// writes the field; a relationship name reads the related value, or list for a `many` relationship. Any other name
/// reads as undefined, where Java throws, because the engine and scripts look up names a value does not have
/// (`toJSON`, `then`, `Symbol.toPrimitive`); writing one is an error, as in Java.
impl HostEntity {
    fn property<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(ctx, || {
            if let Some(found) = prototype_member::<HostEntity>(ctx, &atom)? {
                return Ok(found);
            }
            match atom_name(&atom)? {
                Some(name) => self.read_member(ctx, &name),
                None => Undefined.into_js(ctx),
            }
        })
    }

    fn assign<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>, value: JsValue<'js>) -> Result<bool, JsError> {
        let result = host_guard(ctx, || {
            let Some(name) = atom_name(&atom)? else {
                return Err(Exception::throw_message(
                    ctx,
                    "an entity value field name is not a symbol",
                ));
            };
            self.set_field(ctx, name, &value)?;
            Ok(true)
        });
        keep_assigned(value);
        result
    }

    /// A field is not removed from a value, as Java `EntityValueBase` has no remove.
    fn remove<'js>(&self, _atom: Atom<'js>) -> bool {
        false
    }

    /// `name in value`: a prototype member or a field of the entity.
    fn has_member<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> Result<bool, JsError> {
        if prototype_has::<HostEntity>(ctx, &atom)? {
            return Ok(true);
        }
        Ok(atom_name(&atom)?.is_some_and(|name| self.is_field(&name)))
    }

    fn own_property<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> Result<Option<PropertyDescriptor<'js>>, JsError> {
        let Some(name) = atom_name(&atom)? else {
            return Ok(None);
        };
        if !self.is_field(&name) {
            return Ok(None);
        }
        let held = self.val().get(&name).cloned().unwrap_or(Value::Null);
        let value = host_to_js(ctx, held)?;
        Ok(Some(PropertyDescriptor::new_value(value, true, true, true)))
    }

    /// The fields the value holds, so `Object.keys`, `for...in`, spread and `JSON.stringify` see the field map.
    fn own_names<'js>(&self, ctx: &Ctx<'js>) -> Result<Vec<PropertyName<'js>>, JsError> {
        let names: Vec<String> = self.val().fields().keys().cloned().collect();
        names
            .iter()
            .map(|name| {
                Ok(PropertyName {
                    atom: Atom::from_str(ctx.clone(), name)?,
                    is_enumerable: true,
                })
            })
            .collect()
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

    fn get_entity_name(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(self.val().entity_name().to_string()))
    }

    /// `getMap()`: the fields as a plain map.
    fn get_map<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let map: indexmap::IndexMap<String, Value> = self
                .val()
                .fields()
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            host_to_js(&ctx, Value::Map(map))
        })
    }

    fn set(&self, ctx: Ctx<'_>, field: String, value: JsValue<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || self.set_field(&ctx, field, &value))
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

    /// `getPrimaryKeys()`: the primary key fields of the value, as a map.
    fn get_primary_keys<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            let value = self.val();
            let names: Vec<String> = facade
                .with_entity(value.entity_name(), |entity, _, _| {
                    Ok(entity.pk_field_names().iter().map(|name| (*name).to_string()).collect())
                })
                .map_err(|err| throw_lib(&ctx, &err))?;
            let map: indexmap::IndexMap<String, Value> = names
                .into_iter()
                .map(|name| {
                    let field = value.get(&name).cloned().unwrap_or(Value::Null);
                    (name, field)
                })
                .collect();
            host_to_js(&ctx, Value::Map(map))
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
                    find = find.order_by_text(&moqui_lib::object::to_plain_string(&field));
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
                Some(value) => entity_to_js(&ctx, HostEntity::new(value)),
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

/// A member of the class prototype or of `Object.prototype`. An exotic class's own get runs instead of the
/// prototype lookup, so it looks there itself first.
fn prototype_member<'js, C: rquickjs::class::JsClass<'js>>(
    ctx: &Ctx<'js>,
    atom: &Atom<'js>,
) -> rquickjs::Result<Option<JsValue<'js>>> {
    let Some(prototype) = Class::<C>::prototype(ctx)? else {
        return Ok(None);
    };
    let found: JsValue = prototype.get(atom.clone())?;
    Ok((!found.is_undefined()).then_some(found))
}

fn prototype_has<'js, C: rquickjs::class::JsClass<'js>>(ctx: &Ctx<'js>, atom: &Atom<'js>) -> Result<bool, JsError> {
    match Class::<C>::prototype(ctx)? {
        Some(prototype) => prototype.contains_key(atom.clone()),
        None => Ok(false),
    }
}

/// Hand back the reference to an assigned value that the exotic set took. rquickjs 0.14 wraps the value QuickJS
/// passes to `set_property` as an owned value, and QuickJS frees it again after the call, so without this each
/// assignment would free the value twice. Re-check when rquickjs is upgraded: once the wrapper borrows the value,
/// this would keep one reference too many (a leak, not a crash). The value is not forgotten whole: it holds a
/// reference to the context too.
fn keep_assigned(value: JsValue<'_>) {
    // SAFETY: the context and the value are live for this call; this adds one reference to the value, which the
    // free QuickJS does after the exotic set takes back.
    unsafe {
        rquickjs::qjs::JS_DupValue(value.ctx().as_raw().as_ptr(), value.as_raw());
    }
}

/// The text of a property name, or None for a symbol.
fn atom_name(atom: &Atom<'_>) -> rquickjs::Result<Option<String>> {
    if atom.to_value()?.is_symbol() {
        return Ok(None);
    }
    atom.to_string().map(Some)
}

/// Members of an entity list that are simplest in JavaScript: iteration, Groovy's `size()` and `isEmpty()`, and the
/// array methods the type declarations name. Not enumerable, as the Rust methods are not.
const LIST_PROTOTYPE_JS: &str = r#"(function (list) {
    const define = (name, value) => Object.defineProperty(list, name, { value, writable: true, configurable: true });
    define(Symbol.iterator, function* () { for (let i = 0; i < this.length; i++) yield this.get(i); });
    define("size", function () { return this.length; });
    define("isEmpty", function () { return this.length === 0; });
    define("map", function (callback, self) { return Array.from(this).map(callback, self); });
    define("filter", function (callback, self) { return Array.from(this).filter(callback, self); });
    define("forEach", function (callback, self) { Array.from(this).forEach(callback, self); });
    define("toJSON", function () { return Array.from(this); });
})"#;

/// Add the JavaScript members of the entity list prototype. Lists made after this share it.
pub(super) fn install_entity_prototypes(ctx: &Ctx<'_>) -> Result<(), Error> {
    let list = caught(ctx, "entity list prototype", Class::<HostList>::prototype(ctx))?
        .ok_or_else(|| Error::new("entity list has no prototype"))?;
    let fill: Function = eval_js(ctx, LIST_PROTOTYPE_JS)?;
    caught(ctx, "entity list prototype", fill.call::<_, ()>((list,)))
}

#[cfg(test)]
mod tests {
    use crate::context::{ExecutionContext, ExecutionContextFactory};
    use crate::script::template::{render_trusted, Escape};
    use crate::script::{clear_context_stack, eval_condition, run_script};

    fn holds(expression: &str) {
        assert!(eval_condition(expression).unwrap(), "{expression}");
    }

    /// A context with the in-memory database: types T1 and T2, and enumerations E1 to E3 of T1.
    fn enumerations() -> ExecutionContext {
        clear_context_stack();
        let context = ExecutionContextFactory::test_context();
        context.factory().set_entity(crate::testing::open());
        context
    }

    fn seed() {
        run_script(
            r#"for (const [id, text] of [["T1", "Type 1"], ["T2", "Type 2"]]) {
                const type = ec.entity.makeValue("moqui.basic.EnumerationType");
                type.enumTypeId = id;
                type.description = text;
                type.create();
            }
            for (const [id, text, n] of [["E2", "second", 2], ["E1", "first", 1], ["E3", "third", 3]]) {
                const value = ec.entity.makeValue("moqui.basic.Enumeration");
                value.enumId = id;
                value.enumTypeId = "T1";
                value.description = text;
                value.sequenceNum = n;
                value.create();
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn a_field_reads_and_writes_as_a_property() {
        let context = enumerations();
        let _entered = context.enter();
        holds(
            r#"(function () {
                const value = ec.entity.makeValue("moqui.basic.Enumeration");
                value.enumId = "X1";
                value["description"] = "by index";
                value.sequenceNum = 4;
                return value.get("enumId") === "X1" && value.description === "by index" && value.sequenceNum === 4
                    && value.sequenceNum === value.get("sequenceNum");
            })()"#,
        );
        // A field the value does not hold is null, and optional chaining goes through it.
        holds(
            r#"(function () {
                const value = ec.entity.makeValue("moqui.basic.Enumeration");
                value.set("enumCode", "C");
                return value.parentEnumId === null && value.parentEnumId?.length === undefined && value.enumCode === "C";
            })()"#,
        );
    }

    #[test]
    fn an_unknown_name_reads_undefined_and_does_not_write() {
        let context = enumerations();
        let _entered = context.enter();
        holds(r#"ec.entity.makeValue("moqui.basic.Enumeration").noSuchField === undefined"#);
        holds(r#"ec.entity.makeValue("moqui.basic.Enumeration").get("noSuchField") === null"#);
        for script in [
            r#"ec.entity.makeValue("moqui.basic.Enumeration").noSuchField = 1"#,
            r#"ec.entity.makeValue("moqui.basic.Enumeration").set("noSuchField", 1)"#,
            r#"ec.entity.makeValue("moqui.basic.Enumeration").update = 1"#,
        ] {
            let err = run_script(script).unwrap_err();
            assert!(
                err.message()
                    .contains("is not valid for entity moqui.basic.Enumeration"),
                "{script}: {err}"
            );
        }
    }

    #[test]
    fn a_set_through_the_prototype_chain_defines_the_property_on_the_receiver() {
        let context = enumerations();
        let _entered = context.enter();
        seed();
        holds(
            r#"(function () {
                const value = ec.entity.makeValue("moqui.basic.Enumeration");
                value.enumId = "X1";
                const child = Object.create(value);
                child.enumId = "C1";
                child.noSuchField = 2;
                const other = {};
                Reflect.set(value, "description", "other", other);
                return value.enumId === "X1" && value.get("enumId") === "X1" && child.enumId === "C1"
                    && Object.keys(child).join() === "enumId,noSuchField" && child.noSuchField === 2
                    && value.description === null && other.description === "other"
                    && Reflect.set(value, "enumId", "X2", value) && value.enumId === "X2";
            })()"#,
        );
        holds(
            r#"(function () {
                const list = ec.entity.find("moqui.basic.Enumeration").orderBy("sequenceNum").list();
                const child = Object.create(list);
                child.extra = "e";
                child[0] = "zero";
                list.extra = "ignored";
                return child.extra === "e" && child[0] === "zero" && list[0].enumId === "E1"
                    && list.extra === undefined && child.length === 3;
            })()"#,
        );
    }

    #[test]
    fn a_method_name_wins_over_a_field_name() {
        let context = enumerations();
        let _entered = context.enter();
        holds(
            r#"(function () {
                const value = ec.entity.makeValue("moqui.basic.Enumeration");
                return typeof value.get === "function" && typeof value.findRelatedOne === "function"
                    && typeof value.toString === "function" && "update" in value && "enumId" in value
                    && !("noSuchField" in value) && value[Symbol.iterator] === undefined;
            })()"#,
        );
    }

    #[test]
    fn a_relationship_name_reads_the_related_value_or_list() {
        let context = enumerations();
        let _entered = context.enter();
        seed();
        holds(
            r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "E1").one().type.description === "Type 1""#,
        );
        holds(
            r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "E1").one()["moqui.basic.EnumerationType"].enumTypeId === "T1""#,
        );
        holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "E1").one().parent === null"#);
        holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "E1").one().groupMembers.length === 0"#);
    }

    #[test]
    fn keys_json_spread_and_for_in_see_the_fields() {
        let context = enumerations();
        let _entered = context.enter();
        holds(
            r#"(function () {
                const value = ec.entity.makeValue("moqui.basic.Enumeration");
                value.enumId = "X1";
                value.description = "d";
                const seen = [];
                for (const key in value) seen.push(key);
                const copy = { ...value };
                return Object.keys(value).join() === "description,enumId" && seen.join() === "description,enumId"
                    && JSON.stringify(value) === '{"description":"d","enumId":"X1"}'
                    && copy.enumId === "X1" && Object.keys(copy).length === 2
                    && Object.prototype.hasOwnProperty.call(value, "enumId")
                    && Object.getOwnPropertyDescriptor(value, "description").value === "d";
            })()"#,
        );
    }

    #[test]
    fn a_list_indexes_iterates_and_sizes() {
        let context = enumerations();
        let _entered = context.enter();
        seed();
        holds(
            r#"(function () {
                const list = ec.entity.find("moqui.basic.Enumeration").orderBy("sequenceNum").list();
                const ids = [];
                for (const value of list) ids.push(value.enumId);
                return list[0].enumId === "E1" && list[2].description === "third" && list[3] === undefined
                    && list.length === 3 && list.size() === 3 && !list.isEmpty() && "1" in list && !("3" in list)
                    && ids.join() === "E1,E2,E3" && list.map((value) => value.sequenceNum).join() === "1,2,3"
                    && list.filter((value) => value.sequenceNum > 1).length === 2
                    && JSON.parse(JSON.stringify(list))[1].enumId === "E2" && Object.keys(list).join() === "0,1,2";
            })()"#,
        );
    }

    #[test]
    fn a_template_loop_reads_fields_by_dot() {
        let context = enumerations();
        let _entered = context.enter();
        seed();
        run_script(r#"enumList = ec.entity.find("moqui.basic.Enumeration").orderBy("sequenceNum").list()"#).unwrap();
        let text = render_trusted(
            "<#list enumList as row>${row.enumId}=${row.description} (${row.type.description});</#list>",
            Escape::None,
        )
        .unwrap();
        assert_eq!(text, "E1=first (Type 1);E2=second (Type 1);E3=third (Type 1);");
    }
}
