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

//! `sri`: what the generated screen code calls while a screen renders. The functions here read the
//! [`RenderState`] and the context stack, and run the generated code of a section through a
//! [`Runner`]. The JavaScript bindings are in `script::runtime`.
//!
//! Deferred to the html slices because only their macros call them: `getFieldTransitionValue`
//! (calls a transition as a test render), `makeFormListSingleMap`,
//! `makeFormListMultiMap`, `getFormListHeaderValues`, `getFormFieldValues`, form-list aggregation
//! (`group-by`, `sub-list`, `show-total`), and saved finds.

use std::sync::Arc;

use indexmap::IndexMap;
use moqui_lib::format::{Decimal, RoundingStrategy, ToPrimitive};
use moqui_lib::object::{is_empty, to_plain_string, Value};

use super::definition::{ScreenDef, SectionDef, SectionKind};
use super::form::{self, FormInstance};
use super::generate::{Unit, UnitRequest};
use super::render::{check_condition, Frames, RenderState};
use super::url::ScreenUrlInfo;
use crate::context::{Error, ErrorKind, ExecutionContext};
use crate::{l10n, script, user, web};

/// Generates and runs the code of a section on the script runtime that is running the render.
pub trait Runner {
    /// The generated code for a section, None when it has no widgets.
    ///
    /// # Errors
    ///
    /// Returns an error when the section's code cannot be generated.
    fn generate(&self, request: &UnitRequest<'_>) -> Result<Option<Arc<Unit>>, Error>;

    /// The code for a dynamic form as it was just assembled.
    ///
    /// # Errors
    ///
    /// Returns an error when the form's code cannot be generated.
    fn generate_dynamic(&self, request: &UnitRequest<'_>, form: &FormInstance) -> Result<Arc<Unit>, Error>;

    /// Run a unit on the render's runtime.
    ///
    /// # Errors
    ///
    /// Returns the error the code raised.
    fn run(&self, unit: &Unit) -> Result<(), Error>;
}

/// An element as the generated code passes it: name, attributes, text, children, and the chain of parents.
#[derive(Clone, Debug, Default)]
pub struct NodeLit {
    pub name: String,
    pub attrs: IndexMap<String, String>,
    pub text: String,
    pub children: Vec<NodeLit>,
    pub parent: Option<Box<NodeLit>>,
}

impl NodeLit {
    /// An attribute, empty when it is not there.
    pub fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map_or("", String::as_str)
    }

    /// The element two levels up, which for a widget is its field.
    pub fn field(&self) -> Option<&NodeLit> {
        self.parent.as_ref()?.parent.as_deref()
    }

    pub fn child(&self, name: &str) -> Option<&NodeLit> {
        self.children.iter().find(|child| child.name == name)
    }

    /// The element as XML text.
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        self.write_xml(&mut out);
        out
    }

    fn write_xml(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        for (key, value) in &self.attrs {
            out.push(' ');
            out.push_str(key);
            out.push_str("=\"");
            out.push_str(&moqui_lib::string::encode_for_xml_attribute(value, false));
            out.push('"');
        }
        out.push('>');
        out.push_str(
            &self
                .text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
        );
        for child in &self.children {
            child.write_xml(out);
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

/// The rows of a form-list being rendered: the context frame contents of each row after the row actions.
#[derive(Debug, Default)]
pub struct FormRows {
    pub rows: Vec<IndexMap<String, Value>>,
}

/// `WebUtilities.encodeHtml`.
pub fn encode_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

// ---- output ----

pub fn content_type_header(state: &RenderState) -> String {
    format!("{};charset={}", state.content_type, state.character_encoding)
}

/// Append text to the response. The first write opens it. Nothing is written once a redirect or
/// another response has stopped the render.
pub fn write(state: &RenderState, text: &str) {
    if state.dont_do_render.get() || text.is_empty() {
        return;
    }
    if !web::output_sent() {
        web::start_output(200, &content_type_header(state));
    }
    web::write_output(text);
}

/// Text added with `appendToAfterScreenWriter` and `appendToScriptWriter` that nothing else wrote.
pub fn flush_end_text(state: &RenderState) {
    let after = std::mem::take(&mut *state.after_text.borrow_mut());
    write(state, &after);
    let script_text = std::mem::take(&mut *state.script_text.borrow_mut());
    if !script_text.is_empty() {
        write(state, &format!("\n<script>\n{script_text}\n</script>\n"));
    }
}

pub fn do_boundary_comments(state: &RenderState) -> bool {
    state.screen_path_index.get() != 0 && state.facade.conf().boundary_comments
}

pub fn active_def(state: &RenderState) -> Arc<ScreenDef> {
    if let Some(def) = state.override_def.borrow().as_ref() {
        return Arc::clone(def);
    }
    Arc::clone(&state.url_info.path_defs[state.screen_path_index.get()])
}

/// The path names to the active screen, without the root.
pub fn active_screen_path(state: &RenderState) -> Vec<String> {
    let index = state.screen_path_index.get().min(state.url_info.full_path.len());
    state.url_info.full_path[..index].to_vec()
}

// ---- rendering ----

fn is_security(err: &Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::AuthenticationRequired | ErrorKind::Unauthorized | ErrorKind::Tarpit
    )
}

/// Run a nested render. As in Java, an error other than a security one is logged and its message is
/// written as text where the output would have been, so one section does not lose the page.
fn guarded(state: &RenderState, label: &str, body: impl FnOnce() -> Result<(), Error>) -> Result<(), Error> {
    match body() {
        Err(err) if !is_security(&err) => {
            tracing::error!("Error rendering {label}: {err}");
            write(
                state,
                &encode_html(&format!("Error rendering {label}: {}", err.message())),
            );
            Ok(())
        }
        other => other,
    }
}

/// `ScreenDefinition.render`: the active screen's frame, then its root section.
///
/// # Errors
///
/// Returns an error when the user may not view the screen, or a section raises one.
pub fn render_active_screen(state: &RenderState, runner: &dyn Runner, is_target: bool) -> Result<(), Error> {
    let def = active_def(state);
    render_def(state, runner, &def, is_target)
}

fn render_def(state: &RenderState, runner: &dyn Runner, def: &Arc<ScreenDef>, is_target: bool) -> Result<(), Error> {
    let mut frames = Frames::default();
    frames.enter_screen(def, is_target)?;
    frames.hit_detail(Some(&state.content_type), def.track_artifact_hit);
    render_section_of(state, runner, def, "")
}

/// `sri.renderSection(name)`.
///
/// # Errors
///
/// Returns a security error. Any other error is written as text, as Java does.
pub fn render_section(state: &RenderState, runner: &dyn Runner, name: &str) -> Result<(), Error> {
    let def = active_def(state);
    let label = format!("section [{name}] in screen [{}]", def.location);
    guarded(state, &label, || render_section_of(state, runner, &def, name))
}

/// The screen of a form that one of `def`'s forms `extends`, when that screen has the section `name`: a section
/// inside an extended form's field, which Java adds to the extending screen's sections (`ScreenForm`).
fn extended_section_screen(state: &RenderState, def: &ScreenDef, name: &str) -> Result<Option<Arc<ScreenDef>>, Error> {
    for form in def.forms.values() {
        let Some(extends) = form.node.node()?.attribute("extends") else {
            continue;
        };
        let Some((location, _)) = extends.split_once('#') else {
            continue;
        };
        if location == def.location || location.contains("${") {
            continue;
        }
        let other = state.facade.screen_def(&state.factory, location)?;
        if other.sections.contains_key(name) {
            return Ok(Some(other));
        }
    }
    Ok(None)
}

/// `sri.renderSectionInclude(node)`.
///
/// # Errors
///
/// Same as [`render_section`].
pub fn render_section_include(state: &RenderState, runner: &dyn Runner, include: &NodeLit) -> Result<(), Error> {
    let location = include.attr("location");
    let name = include.attr("name");
    if location.contains("${") || name.contains("${") {
        let def = active_def(state);
        let label = format!("section include [{location}#{name}] in screen [{}]", def.location);
        return guarded(state, &label, || {
            let location = script::expand(location)?;
            let name = script::expand(name)?;
            let other = state.facade.screen_def(&state.factory, &location)?;
            let previous = state.override_def.replace(Some(Arc::clone(&other)));
            let result = render_section_of(state, runner, &other, &name);
            state.override_def.replace(previous);
            result
        });
    }
    let name = match (name.is_empty(), location.split_once('#')) {
        (true, Some((_, section))) => section,
        _ => name,
    };
    render_section(state, runner, name)
}

fn render_section_of(state: &RenderState, runner: &dyn Runner, def: &Arc<ScreenDef>, name: &str) -> Result<(), Error> {
    if !name.is_empty() && !def.sections.contains_key(name) {
        if let Some(other) = extended_section_screen(state, def, name)? {
            let previous = state.override_def.replace(Some(Arc::clone(&other)));
            let result = render_section_of(state, runner, &other, name);
            state.override_def.replace(previous);
            return result;
        }
    }
    let section = if name.is_empty() {
        &def.root
    } else {
        def.sections
            .get(name)
            .ok_or_else(|| Error::new(format!("No section with name [{name}] in screen [{}]", def.location)))?
    };
    if let SectionKind::Include { location, name } = &section.kind {
        let request = NodeLit {
            name: "section-include".to_string(),
            attrs: IndexMap::from([
                ("location".to_string(), location.clone()),
                ("name".to_string(), name.clone()),
            ]),
            ..NodeLit::default()
        };
        return render_section_include(state, runner, &request);
    }
    match &section.kind {
        SectionKind::Iterate {
            list,
            entry,
            key,
            paginate,
        } => {
            let value = script::eval_value(list)?;
            if is_empty(&value) {
                return Ok(());
            }
            let items = iteration_entries(&value);
            let paginated = *paginate;
            let mut items = items;
            if paginated {
                script::push_context_frame();
                items = match paginate_items(list, items) {
                    Ok(page) => page,
                    Err(err) => {
                        script::pop_context_frame();
                        return Err(err);
                    }
                };
            }
            let result = iterate_section(state, runner, def, section, name, entry, key.as_deref(), &items);
            if paginated {
                script::pop_context_frame();
            }
            result
        }
        _ => render_single(state, runner, def, section, name),
    }
}

/// The entries of a section-iterate list: each item, and for a map its key and value.
pub(super) fn iteration_entries(value: &Value) -> Vec<(Option<String>, Value)> {
    match value {
        Value::Map(entries) => entries
            .iter()
            .map(|(key, item)| (Some(key.clone()), item.clone()))
            .collect(),
        other => script::value_items(other)
            .unwrap_or_default()
            .into_iter()
            .map(|item| (None, item))
            .collect(),
    }
}

/// `CollectionUtilities.paginateList`: the page of `items` named by `pageIndex` and `pageSize`, with
/// the list's count and page variables put in the context.
fn paginate_items(list_name: &str, items: Vec<(Option<String>, Value)>) -> Result<Vec<(Option<String>, Value)>, Error> {
    let mut context = script::context_combined();
    let size = i32::try_from(items.len()).map_err(|_| Error::new("list is too long to paginate"))?;
    moqui_lib::collection::paginate_parameters(size, list_name, &mut context)?;
    for suffix in [
        "Count",
        "PageIndex",
        "PageSize",
        "PageMaxIndex",
        "PageRangeLow",
        "PageRangeHigh",
    ] {
        let name = format!("{list_name}{suffix}");
        if let Some(value) = context.get(&name) {
            script::set_context_name(&name, value.clone())?;
        }
    }
    let number = |suffix: &str| {
        context
            .get(&format!("{list_name}{suffix}"))
            .map(to_plain_string)
            .and_then(|text| text.parse::<usize>().ok())
            .unwrap_or(0)
    };
    let (low, high) = (number("PageRangeLow"), number("PageRangeHigh"));
    let start = low.saturating_sub(1).min(items.len());
    let end = high.min(items.len()).max(start);
    Ok(items[start..end].to_vec())
}

#[expect(clippy::too_many_arguments)]
fn iterate_section(
    state: &RenderState,
    runner: &dyn Runner,
    def: &Arc<ScreenDef>,
    section: &SectionDef,
    name: &str,
    entry: &str,
    key: Option<&str>,
    items: &[(Option<String>, Value)],
) -> Result<(), Error> {
    for (index, (item_key, item)) in items.iter().enumerate() {
        script::push_context_frame();
        let _ = script::set_context_name(entry, item.clone());
        if let (Some(key_name), Some(item_key)) = (key, item_key) {
            let _ = script::set_context_name(key_name, Value::string(item_key));
        }
        let _ = script::set_context_name("sectionEntryIndex", Value::I64(index as i64));
        let _ = script::set_context_name(&format!("{entry}_index"), Value::I64(index as i64));
        let _ = script::set_context_name(&format!("{entry}_has_next"), Value::Bool(index + 1 < items.len()));
        let result = render_single(state, runner, def, section, name);
        script::pop_context_frame();
        result?;
        if state.dont_do_render.get() {
            break;
        }
    }
    Ok(())
}

fn render_single(
    state: &RenderState,
    runner: &dyn Runner,
    def: &Arc<ScreenDef>,
    section: &SectionDef,
    name: &str,
) -> Result<(), Error> {
    let holds = match &section.condition {
        Some(condition) => check_condition(condition)?,
        None => true,
    };
    let fail = !holds;
    if holds {
        if let Some(actions) = &section.actions {
            script::run_actions(&actions.xml)?;
        }
        if state.dont_do_render.get() {
            return Ok(());
        }
    }
    let request = UnitRequest {
        factory: &state.factory,
        facade: &state.facade,
        def,
        section: name,
        fail,
        mode: &state.render_mode,
        macro_location: &state.macro_location,
    };
    let Some(unit) = runner.generate(&request)? else {
        return Ok(());
    };
    let errors = ExecutionContext::with_current(|ec| ec.message().errors().join("\n")).unwrap_or_default();
    if holds && !errors.is_empty() {
        // Rendering widgets after an error in the actions would only add more errors.
        write(state, &encode_html(&errors));
        return Ok(());
    }
    runner.run(&unit)
}

/// `sri.renderSubscreen()`: the next screen in the path.
///
/// # Errors
///
/// Returns a security error, and the error of a subscreen that is not available.
pub fn render_subscreen(state: &RenderState, runner: &dyn Runner) -> Result<(), Error> {
    let defs = &state.url_info.path_defs;
    let index = state.screen_path_index.get();
    if index + 1 >= defs.len() {
        // include-child-content: the file under the target screen is the subscreen, rendered as a template.
        if let Some(file) = &state.url_info.file_resource {
            let text = crate::script::template::TemplateRegistry::of(&state.factory).render_location_default(
                &state.factory,
                file,
                None,
                "",
            )?;
            write(state, &text);
            return Ok(());
        }
        let def = active_def(state);
        write(
            state,
            &encode_html(&format!(
                "Tried to render subscreen in screen [{}] but there is no subscreens.@default-item, and no more valid subscreen names in the screen path [{:?}]",
                def.location, state.url_info.full_path
            )),
        );
        return Ok(());
    }
    let next = Arc::clone(&defs[index + 1]);
    let item = state
        .url_info
        .full_path
        .get(index)
        .and_then(|name| active_def(state).subscreens.get(name).cloned());
    // An item limited to a user group is there only for its members, at any depth.
    if item.as_ref().is_some_and(|item| !item.is_valid_in_current_context()) {
        return Err(Error::of_kind(
            ErrorKind::Unauthorized,
            format!("The screen {} is not available", next.screen_name),
        ));
    }
    // `disable-when` blocks the screen below the first level. (Java defines it but reads it nowhere; kept here.)
    let disabled = match item.as_ref().and_then(|item| item.disable_when.as_ref()) {
        Some(disable) if index > 0 => script::eval_condition(disable)?,
        _ => false,
    };
    if disabled {
        return Err(Error::of_kind(
            ErrorKind::Unauthorized,
            format!("The screen {} is not available", next.screen_name),
        ));
    }
    state.screen_path_index.set(index + 1);
    let result = if state.stopped.borrow().contains(&next.location) {
        Ok(())
    } else {
        let is_target = index + 2 >= defs.len();
        guarded(state, &format!("screen [{}]", next.location), || {
            render_def(state, runner, &next, is_target)
        })
    };
    state.screen_path_index.set(index);
    result
}

/// `sri.renderIncludeScreen(location, shareScope)`.
///
/// # Errors
///
/// Returns a security error. Any other error is written as text.
pub fn render_include_screen(
    state: &RenderState,
    runner: &dyn Runner,
    location: &str,
    share_scope: bool,
) -> Result<(), Error> {
    if !share_scope {
        script::push_context_frame();
    }
    let previous = state.override_def.borrow().clone();
    let result = guarded(state, &format!("screen [{location}]"), || {
        let def = state.facade.screen_def(&state.factory, location)?;
        *state.override_def.borrow_mut() = Some(Arc::clone(&def));
        render_def(state, runner, &def, false)
    });
    *state.override_def.borrow_mut() = previous;
    if !share_scope {
        script::pop_context_frame();
    }
    result
}

/// `sri.renderText(location, template)`. With template `false` the file text is returned. Otherwise
/// the renderer registered for the file extension renders it with the live context and the result is
/// written, and the return is empty. A location whose extension has no renderer is a trusted fancy
/// template. FreeMarker files are not rendered.
///
/// # Errors
///
/// Returns an error when the file cannot be read, is a FreeMarker template, or fails to render.
pub fn render_text(state: &RenderState, location: &str, template: &str) -> Result<String, Error> {
    if location.is_empty() || location == "null" {
        tracing::warn!(
            "Not rendering text in screen [{}], location was empty",
            active_def(state).location
        );
        return Ok(String::new());
    }
    let registry = script::template::TemplateRegistry::of(&state.factory);
    if template == "false" {
        return registry
            .location_text(&state.factory, location)
            .map(|text| text.to_string());
    }
    if location.ends_with(".ftl") {
        return Err(Error::new(format!(
            "FreeMarker template {location} is not supported; port it to the fancy dialect"
        )));
    }
    // As Java's renderText: the renderer of the location's extension, else of the `template` value as an extension,
    // else the file's text as it is, so a .js or .css file is not run as a template.
    script::push_context_frame();
    let rendered = registry.render_location_default(&state.factory, location, None, template);
    script::pop_context_frame();
    write(state, &rendered?);
    Ok(String::new())
}

pub fn append_to_after_screen_writer(state: &RenderState, text: &str) {
    state.after_text.borrow_mut().push_str(text);
}

pub fn append_to_script_writer(state: &RenderState, text: &str) {
    state.script_text.borrow_mut().push_str(text);
}

// ---- field values ----

/// `sri.getFieldValue(field, default)`: the form's error parameters, the field's `from`, the form's
/// map, then the context, then the expanded default.
///
/// # Errors
///
/// Returns an error when an expression in the form does not evaluate.
pub fn get_field_value(field: &NodeLit, default_value: &str) -> Result<Value, Error> {
    let field_name = field.attr("name");
    let mut value = Value::Null;
    if let Some(form) = field.parent.as_deref() {
        if form.name == "form-single" {
            let errors = web::saved_for_page().error_parameters;
            if errors.get("moquiFormName").map(String::as_str) == Some(form.attr("name")) {
                if let Some(found) = errors.get(field_name).filter(|text| !text.is_empty()) {
                    return Ok(Value::string(found));
                }
            }
            let from = if field.attr("from").is_empty() {
                field.attr("entry-name")
            } else {
                field.attr("from")
            };
            if !from.is_empty() {
                return script::eval_value(from);
            }
            let map_name = if form.attr("map").is_empty() {
                "fieldValues"
            } else {
                form.attr("map")
            };
            let map = script::eval_value(map_name).unwrap_or(Value::Null);
            if let Some(found) = script::value_field(&map, field_name) {
                value = found;
            }
        }
    }
    if is_empty(&value) {
        let from_context = script::context_value(field_name);
        if let Some(found) = from_context.filter(|found| !is_empty(found)) {
            return Ok(found);
        }
    } else {
        return Ok(value);
    }
    let default_text = script::expand(default_value)?;
    if !default_text.is_empty() {
        return Ok(Value::String(default_text));
    }
    Ok(value)
}

fn format_for_mode<'a>(state: &RenderState, widget: &'a NodeLit) -> Option<&'a str> {
    let mut format = widget.attr("format");
    if state.render_mode == "text" || state.render_mode == "csv" {
        let text_format = widget.attr("text-format");
        if !text_format.is_empty() {
            format = text_format;
        }
    }
    Some(format).filter(|text| !text.is_empty())
}

/// `sri.getFieldValueString(widget)`.
///
/// # Errors
///
/// Returns an error when the value or its format cannot be made.
pub fn get_field_value_string(state: &RenderState, widget: &NodeLit) -> Result<String, Error> {
    let field = widget.field().ok_or_else(|| Error::new("widget has no field"))?;
    let value = get_field_value(field, widget.attr("default-value"))?;
    value_string(&value, format_for_mode(state, widget))
}

/// `sri.getFieldValueString(field, default, format, type)`. A date-time widget names its `type`, so a
/// value that reached the script as text or as milliseconds is formatted as the date, time or time stamp it is.
///
/// # Errors
///
/// Returns an error when the value or its format cannot be made.
pub fn get_field_value_string_for(
    field: &NodeLit,
    default_value: &str,
    format: &str,
    kind: &str,
) -> Result<String, Error> {
    let value = get_field_value(field, default_value)?;
    let format = Some(format).filter(|text| !text.is_empty());
    if let Value::String(text) = &value {
        if !text.is_empty() && !kind.is_empty() {
            let zone = l10n::time_zone();
            let parsed = match kind {
                "date" => moqui_lib::object::parse_date(text).map(Value::Date),
                "time" => moqui_lib::object::parse_time(text).map(Value::Time),
                _ => moqui_lib::object::parse_timestamp(text, &zone).map(Value::Timestamp),
            };
            if let Some(parsed) = parsed {
                return l10n::format_value(&parsed, format);
            }
        }
    }
    value_string(&value, format)
}

fn value_string(value: &Value, format: Option<&str>) -> Result<String, Error> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        other => l10n::format_value(other, format),
    }
}

/// `sri.getFieldValuePlainString(field, default)`.
///
/// # Errors
///
/// Returns an error when the value cannot be made.
pub fn get_field_value_plain_string(field: &NodeLit, default_value: &str) -> Result<String, Error> {
    let value = get_field_value(field, "")?;
    if is_empty(&value) && !default_value.is_empty() {
        return script::expand(default_value);
    }
    Ok(to_plain_string(&value))
}

/// `sri.getFieldValueClass(field)`.
///
/// # Errors
///
/// Returns an error when the value cannot be made.
pub fn get_field_value_class(field: &NodeLit) -> Result<String, Error> {
    Ok(match get_field_value(field, "")? {
        Value::String(_) | Value::Null => "String",
        Value::Bool(_) => "Boolean",
        Value::I32(_) => "Integer",
        Value::I64(_) => "Long",
        Value::F64(_) => "Double",
        Value::Decimal(_) => "BigDecimal",
        Value::Timestamp(_) => "Timestamp",
        Value::Date(_) => "Date",
        Value::Time(_) => "Time",
        Value::Bytes(_) => "byte[]",
        Value::List(_) => "ArrayList",
        Value::Map(_) => "LinkedHashMap",
        Value::Object(_) => "EntityValue",
    }
    .to_string())
}

/// `sri.getNamedValuePlain(fieldName, formNode)`.
///
/// # Errors
///
/// Returns an error when the form's map expression does not evaluate.
pub fn get_named_value_plain(field_name: &str, form: &NodeLit) -> Result<String, Error> {
    let mut value = None;
    if form.name == "form-single" {
        let map_name = if form.attr("map").is_empty() {
            "fieldValues"
        } else {
            form.attr("map")
        };
        let map = script::eval_value(map_name)?;
        value = script::value_field(&map, field_name);
    }
    let value = value.or_else(|| script::context_value(field_name));
    Ok(value.map(|found| to_plain_string(&found)).unwrap_or_default())
}

fn default_text(widget: &NodeLit) -> Result<String, Error> {
    let text = widget.attr("default-text");
    if text.is_empty() {
        Ok(String::new())
    } else {
        script::expand_localized(text)
    }
}

/// `sri.getFieldEntityValue(widget)`: the text of the entity value the field's value points at.
///
/// # Errors
///
/// Returns an error when the entity cannot be found or read.
pub fn get_field_entity_value(widget: &NodeLit) -> Result<String, Error> {
    let field = widget.field().ok_or_else(|| Error::new("widget has no field"))?;
    let field_value = get_field_value(field, "")?;
    if field_value.is_null() {
        return default_text(widget);
    }
    let entity_name = widget.attr("entity-name");
    let shape = form::entity_shape(entity_name)?;
    let mut key_field = widget.attr("key-field-name");
    if key_field.is_empty() {
        key_field = widget.attr("entity-key-name");
    }
    let key_field = if key_field.is_empty() {
        shape.pk_fields.first().cloned().unwrap_or_default()
    } else {
        key_field.to_string()
    };
    let facade = ExecutionContext::with_current(|ec| ec.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("no entity facade is installed"))?;
    // As Java's getFieldEntityValue: through the entity cache unless the widget says `use-cache="false"`, so a form-list
    // does not query once per row for the same few values.
    let use_cache = [widget.attr("use-cache"), widget.attr("entity-use-cache")]
        .into_iter()
        .find(|text| !text.is_empty())
        .is_none_or(|text| text == "true");
    let found = facade
        .find(entity_name)?
        .eq(&key_field, field_value)
        .use_cache(use_cache)
        .one()?;
    let Some(found) = found else {
        return default_text(widget);
    };
    let text = widget.attr("text");
    if !text.is_empty() {
        let fields: IndexMap<String, Value> = found.fields().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        script::push_context_frame();
        script::put_context_all(&fields);
        let expanded = script::expand_localized(text);
        script::pop_context_frame();
        return expanded;
    }
    if shape.description_field.is_empty() {
        return Ok(String::new());
    }
    Ok(found
        .get(&shape.description_field)
        .map(to_plain_string)
        .unwrap_or_default())
}

// ---- options ----

/// `sri.getFieldOptions(widget)`: the options of a drop-down, radio or check in order.
///
/// # Errors
///
/// Returns an error when an `entity-find` or `list-options` expression fails.
pub fn get_field_options(state: &RenderState, widget: &NodeLit) -> Result<Vec<(String, String)>, Error> {
    let field_name = widget.field().map_or("", |field| field.attr("name"));
    let mut options: Vec<(String, String)> = Vec::new();
    let mut put = |key: String, text: String| match options.iter_mut().find(|(existing, _)| *existing == key) {
        Some(slot) => slot.1 = text,
        None => options.push((key, text)),
    };
    for child in &widget.children {
        match child.name.as_str() {
            "entity-options" => {
                let Some(find) = child.child("entity-find") else {
                    continue;
                };
                for item in find_list(find)? {
                    if let Some((key, text)) = field_option(field_name, child, &item)? {
                        put(key, text);
                    }
                }
            }
            "list-options" => {
                let list = script::eval_value(child.attr("list"))?;
                let has_key_or_text = !child.attr("key").is_empty() || !child.attr("text").is_empty();
                for item in script::value_items(&list).unwrap_or_default() {
                    if script::value_map(&item).is_some() {
                        if let Some((key, text)) = field_option(field_name, child, &item)? {
                            put(key, text);
                        }
                    } else if has_key_or_text {
                        let wrapped = Value::Map(IndexMap::from([("entry".to_string(), item)]));
                        if let Some((key, text)) = field_option(field_name, child, &wrapped)? {
                            put(key, text);
                        }
                    } else {
                        let key = to_plain_string(&item);
                        let text = l10n::localize(&key)?;
                        put(key, text);
                    }
                }
            }
            "option" => {
                let mut key = child.attr("key").to_string();
                if key.contains("${") {
                    key = script::expand(&key)?;
                }
                let text = if child.attrs.contains_key("text") {
                    script::expand_localized(child.attr("text"))?
                } else {
                    l10n::localize(&key)?
                };
                put(key, text);
            }
            _ => {}
        }
    }
    if options.is_empty() {
        if let Some(dynamic) = widget.child("dynamic-options") {
            options = dynamic_options(state, dynamic)?;
        }
    }
    Ok(options)
}

/// The options of a `dynamic-options` element, from its transition called as a test render with the
/// `depends-on` values as parameters. It gives no options while a `depends-on` field is empty, and
/// none with `server-search`, where the browser asks as the user types. The transition's JSON is a
/// list of maps, or a map with an `options` list, each with the value and label fields.
fn dynamic_options(state: &RenderState, node: &NodeLit) -> Result<Vec<(String, String)>, Error> {
    if node.attr("server-search") == "true" {
        return Ok(Vec::new());
    }
    let field = |name: &str, default: &'static str| {
        let given = node.attr(name);
        if given.is_empty() {
            default.to_string()
        } else {
            given.to_string()
        }
    };
    let (label_field, value_field) = (field("label-field", "label"), field("value-field", "value"));
    let (parameters, has_all_depends) = node_parameters_checked(node)?;
    if !has_all_depends {
        return Ok(Vec::new());
    }
    let url = super::urls::build_url(state, node.attr("transition"))?;
    let root = state
        .url_info
        .path_defs
        .first()
        .map(|def| def.location.clone())
        .ok_or_else(|| Error::new("no root screen"))?;
    let parameters: std::collections::BTreeMap<String, Value> = parameters.into_iter().collect();
    let rendered = super::screen_test::ScreenTest::new(&state.factory)
        .root_screen(root)
        .render(&url.path_with_params(), parameters)?;
    let json = rendered.json_value()?;
    let list = match &json {
        Value::List(items) => items.clone(),
        Value::Map(entries) => match entries.get("options") {
            Some(Value::List(items)) => items.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    let mut options: Vec<(String, String)> = Vec::new();
    for entry in &list {
        let Value::Map(entry) = entry else {
            continue;
        };
        let value = entry.get(&value_field).map(to_plain_string).unwrap_or_default();
        let label = entry.get(&label_field).map(to_plain_string).unwrap_or_default();
        if !value.is_empty() && !label.is_empty() {
            match options.iter_mut().find(|(key, _)| *key == value) {
                Some(slot) => slot.1 = label,
                None => options.push((value, label)),
            }
        }
    }
    Ok(options)
}

/// Run an `entity-find` element as actions and return the rows.
fn find_list(find: &NodeLit) -> Result<Vec<Value>, Error> {
    const LIST: &str = "fieldOptionList__";
    let mut find = find.clone();
    find.attrs.insert("list".to_string(), LIST.to_string());
    let xml = format!("<actions>{}</actions>", find.to_xml());
    script::push_context_frame();
    let ran = script::run_actions(&xml);
    let list = script::context_value(LIST);
    script::pop_context_frame();
    ran?;
    Ok(list.and_then(|value| script::value_items(&value)).unwrap_or_default())
}

/// `ScreenForm.addFieldOption`: the key and text of one option, or None when it has no key.
fn field_option(field_name: &str, option: &NodeLit, item: &Value) -> Result<Option<(String, String)>, Error> {
    script::push_context_frame();
    let result = field_option_in(field_name, option, item);
    script::pop_context_frame();
    result
}

fn field_option_in(field_name: &str, option: &NodeLit, item: &Value) -> Result<Option<(String, String)>, Error> {
    let fields = script::value_map(item).unwrap_or_default();
    script::put_context_all(&fields);
    let entity_name = script::value_entity_name(item);
    let mut key = None;
    let key_attr = option.attr("key");
    if key_attr.is_empty() {
        if let Some(entity_name) = &entity_name {
            let shape = form::entity_shape(entity_name)?;
            if let Some(first) = shape.pk_fields.first() {
                key = fields.get(first).map(to_plain_string);
            }
        }
    } else {
        let expanded = script::expand(key_attr)?;
        key = Some(expanded).filter(|text| text != "null");
    }
    if key.is_none() {
        key = script::context_value(field_name).map(|value| to_plain_string(&value));
    }
    let Some(key) = key else {
        return Ok(None);
    };
    let text_attr = option.attr("text");
    let text = if text_attr.is_empty() {
        let has_description = match &entity_name {
            Some(name) => form::entity_shape(name)?
                .fields
                .iter()
                .any(|(field, _, _)| field == "description"),
            None => true,
        };
        match fields.get("description").filter(|_| has_description) {
            Some(description) if !description.is_null() => to_plain_string(description),
            _ => l10n::localize(&key)?,
        }
    } else {
        let expanded = script::expand_localized(text_attr)?;
        if expanded == "null" {
            l10n::localize(&key)?
        } else {
            expanded
        }
    };
    Ok(Some((key, text)))
}

// ---- forms ----

fn form_of(state: &RenderState, name: &str) -> Result<Arc<FormInstance>, Error> {
    if let Some(found) = state.dynamic_forms.borrow().get(name) {
        return Ok(Arc::clone(found));
    }
    let def = active_def(state);
    form::assemble(&state.facade, &state.factory, &def, name)
}

/// `sri.renderDynamicForm(name)`: assemble a `dynamic="true"` form from the context as it is now, then
/// generate its code (once for each different result) and run it.
///
/// # Errors
///
/// Returns a security error. Any other error is written as text.
pub fn render_dynamic_form(state: &RenderState, runner: &dyn Runner, name: &str) -> Result<(), Error> {
    let def = active_def(state);
    let label = format!("form [{name}] in screen [{}]", def.location);
    guarded(state, &label, || {
        let form = form::assemble_dynamic(&state.facade, &state.factory, &def, name)?;
        let request = UnitRequest {
            factory: &state.factory,
            facade: &state.facade,
            def: &def,
            section: "",
            fail: false,
            mode: &state.render_mode,
            macro_location: &state.macro_location,
        };
        let unit = runner.generate_dynamic(&request, &form)?;
        let previous = state.dynamic_forms.borrow_mut().insert(name.to_string(), form);
        let result = runner.run(&unit);
        match previous {
            Some(previous) => state.dynamic_forms.borrow_mut().insert(name.to_string(), previous),
            None => state.dynamic_forms.borrow_mut().remove(name),
        };
        result
    })
}

/// `sri.getUserFormListFinds(name)`: the saved finds the user may use for a form of the active screen.
///
/// # Errors
///
/// Returns an error when the saved find records cannot be read.
pub fn user_form_list_finds(state: &RenderState, name: &str) -> Result<Vec<Value>, Error> {
    let def = active_def(state);
    let Some(form) = def.forms.get(name) else {
        return Ok(Vec::new());
    };
    let location = super::saved_finds::form_location(&def.location, form.list, name);
    super::saved_finds::user_form_list_finds(&super::saved_finds::saved_find_location(&location, form.dynamic))
}

/// `sri.getUserDefaultFormListFindId()`: the user's default saved find of the active screen.
///
/// # Errors
///
/// Returns an error when the record cannot be read.
pub fn user_default_form_list_find_id(state: &RenderState) -> Result<Option<String>, Error> {
    super::saved_finds::user_default_find_id(&active_def(state).location)
}

/// `sri.renderUserColumnsForm(name)`: a form-list whose columns the user chose (`FormConfig`, through the column dialog
/// or the active saved find) is assembled with those columns, then generated and run as a dynamic form is. Returns
/// false, rendering nothing, when the user has no columns for it, so the form's own code runs.
///
/// # Errors
///
/// Returns a security error. Any other error is written as text.
pub fn render_user_columns_form(state: &RenderState, runner: &dyn Runner, name: &str) -> Result<bool, Error> {
    let def = active_def(state);
    let Some(form_def) = def.forms.get(name) else {
        return Ok(false);
    };
    let location = super::saved_finds::form_location(&def.location, form_def.list, name);
    let saved = super::saved_finds::saved_find_location(&location, form_def.dynamic);
    let Some(columns) = super::saved_finds::user_columns(&location, &saved)? else {
        return Ok(false);
    };
    let label = format!("form [{name}] in screen [{}]", def.location);
    let mut rendered = false;
    guarded(state, &label, || {
        let base = if form_def.dynamic {
            form::assemble_dynamic(&state.facade, &state.factory, &def, name)?
        } else {
            form::assemble(&state.facade, &state.factory, &def, name)?
        };
        let node = base.node()?;
        let fields: Vec<String> = node
            .children_named("field")
            .iter()
            .filter_map(|field| field.attribute("name"))
            .collect();
        node.remove_where(|child| {
            child.name() == "form-list-column"
                || (child.name() == "columns" && child.attribute("type").is_none_or(|kind| kind.is_empty()))
        });
        let config = moqui_lib::node::MNode::new("columns");
        for column in &columns {
            let column_node = config.append("column");
            for field in column.iter().filter(|field| fields.contains(field)) {
                column_node.append("field-ref").set_attribute("name", field.clone());
            }
        }
        node.append_node(config);
        let form = Arc::new(form::FormInstance::from_node(&def, &node)?);
        let request = UnitRequest {
            factory: &state.factory,
            facade: &state.facade,
            def: &def,
            section: "",
            fail: false,
            mode: &state.render_mode,
            macro_location: &state.macro_location,
        };
        let unit = runner.generate_dynamic(&request, &form)?;
        let previous = state.dynamic_forms.borrow_mut().insert(name.to_string(), form);
        let result = runner.run(&unit);
        match previous {
            Some(previous) => state.dynamic_forms.borrow_mut().insert(name.to_string(), previous),
            None => state.dynamic_forms.borrow_mut().remove(name),
        };
        rendered = true;
        result
    })?;
    Ok(rendered)
}

/// `sri.getFormInstance(name)`: the assembled form.
///
/// # Errors
///
/// Returns an error when the form is not in the screen.
pub fn get_form(state: &RenderState, name: &str) -> Result<Arc<FormInstance>, Error> {
    form_of(state, name)
}

/// `sri.pushSingleFormMapContext(map)`: a frame holding the map's fields and `_formMap`.
pub fn push_single_form_map_context(map: &Value) {
    let fields = script::value_map(map).unwrap_or_default();
    script::push_context_frame();
    script::put_context_all(&fields);
    let _ = script::set_context_name("_formMap", Value::Map(fields));
}

/// `sri.openFormList(name)`: run the form's list and row actions. Returns the handle and the row count.
///
/// # Errors
///
/// Returns an error when the form, its find, its row actions or a field `from` fails.
pub fn open_form_list(state: &RenderState, name: &str) -> Result<(usize, usize), Error> {
    let form = form_of(state, name)?;
    let rows = form_rows(&form)?;
    let count = rows.len();
    let mut lists = state.form_lists.borrow_mut();
    lists.push(Some(FormRows { rows }));
    Ok((lists.len() - 1, count))
}

/// The rows of a form-list. Each row is the context frame after `row-actions` and the fields' `from`
/// expressions have run on the entry, as `AggregationUtil.aggregateList` makes them.
fn form_rows(form: &FormInstance) -> Result<Vec<IndexMap<String, Value>>, Error> {
    let list_value = match &form.entity_find {
        Some(find) => {
            script::run_actions(&find.xml)?;
            script::context_value(&form.list_name).unwrap_or(Value::Null)
        }
        None => script::eval_value(&form.list_name)?,
    };
    let items = script::value_items(&list_value).unwrap_or_default();
    let count = items.len();
    let mut rows = Vec::with_capacity(count);
    for (index, item) in items.into_iter().enumerate() {
        let fields = script::value_map(&item).unwrap_or_default();
        script::push_context_frame();
        script::put_context_all(&fields);
        let index_value = Value::I64(index as i64);
        let has_next = Value::Bool(index + 1 < count);
        let prefix = if form.list_entry.is_empty() {
            &form.list_name
        } else {
            &form.list_entry
        };
        if form.list_entry.is_empty() {
            let _ = script::set_context_name(&format!("{prefix}_entry"), item.clone());
        } else {
            let _ = script::set_context_name(&form.list_entry, item.clone());
        }
        let _ = script::set_context_name(&format!("{prefix}_index"), index_value);
        let _ = script::set_context_name(&format!("{prefix}_has_next"), has_next);
        let ran = (|| -> Result<(), Error> {
            if let Some(actions) = &form.row_actions {
                script::run_actions(&actions.xml)?;
            }
            for (field, from) in &form.field_from {
                let value = script::eval_value(from)?;
                script::set_context_name(field, value)?;
            }
            Ok(())
        })();
        let row = script::pop_context_frame();
        ran?;
        rows.push(row);
    }
    if let Some(aggregation) = &form.aggregation {
        rows = aggregation.apply(rows, |row, name| {
            row.get(name)
                .cloned()
                .or_else(|| script::context_value(name))
                .unwrap_or(Value::Null)
        });
    }
    Ok(rows)
}

/// `sri.formListSubRowCount(info, index)`: the number of sub-list rows of a row of an aggregated form-list.
pub fn form_list_sub_row_count(state: &RenderState, handle: usize, index: usize) -> usize {
    let lists = state.form_lists.borrow();
    match lists
        .get(handle)
        .and_then(Option::as_ref)
        .and_then(|list| list.rows.get(index))
        .and_then(|row| row.get("aggregateSubList"))
    {
        Some(Value::List(items)) => items.len(),
        _ => 0,
    }
}

/// `sri.startFormListSubRow(info, index, subIndex)`: a frame over the row's frame holding the sub-list entry and `_formMap`,
/// as Java's `startFormListSubRow`. `sri.endFormListSubRow()` pops it.
///
/// # Errors
///
/// Returns an error for a row or a sub-row that is not there.
pub fn start_form_list_sub_row(state: &RenderState, handle: usize, index: usize, sub: usize) -> Result<(), Error> {
    let entry = {
        let lists = state.form_lists.borrow();
        let row = lists
            .get(handle)
            .and_then(Option::as_ref)
            .and_then(|list| list.rows.get(index))
            .ok_or_else(|| Error::new("form-list row is not available"))?;
        match row.get("aggregateSubList") {
            Some(Value::List(items)) => items.get(sub).cloned(),
            _ => None,
        }
        .ok_or_else(|| Error::new("form-list sub-row is not available"))?
    };
    let fields = script::value_map(&entry).unwrap_or_default();
    script::push_context_frame();
    script::put_context_all(&fields);
    let _ = script::set_context_name("_formMap", Value::Map(fields));
    Ok(())
}

/// `sri.startFormListRow`: a frame holding the row and `_formMap`.
///
/// # Errors
///
/// Returns an error for a list that was closed or a row past the end.
pub fn start_form_list_row(state: &RenderState, handle: usize, index: usize) -> Result<(), Error> {
    let row = {
        let lists = state.form_lists.borrow();
        lists
            .get(handle)
            .and_then(Option::as_ref)
            .and_then(|list| list.rows.get(index))
            .cloned()
            .ok_or_else(|| Error::new("form-list row is not available"))?
    };
    script::push_context_frame();
    script::put_context_all(&row);
    let _ = script::set_context_name("_formMap", Value::Map(row));
    Ok(())
}

/// `sri.safeCloseList`: drop a form-list's rows.
pub fn close_form_list(state: &RenderState, handle: usize) {
    if let Some(slot) = state.form_lists.borrow_mut().get_mut(handle) {
        *slot = None;
    }
}

/// `FormListRenderInfo.getFormListColumnCharWidths`: the width of each column in characters. `columns`
/// holds the `print-width` and `print-width-type` of each field of each column.
///
/// # Errors
///
/// Returns an error when fixed widths exceed the line, or a width is not a number.
pub fn form_list_column_char_widths(columns: &[Vec<(String, String)>], line_width: i64) -> Result<Vec<i64>, Error> {
    let columns_count = columns.len();
    if columns_count == 0 {
        return Ok(Vec::new());
    }
    let original = if line_width == 0 { 132 } else { line_width };
    let mut line = original - (i64::try_from(columns_count).unwrap_or(0) - 1);
    let mut char_widths: Vec<Option<i64>> = vec![None; columns_count];
    let mut percent_widths: Vec<Option<Decimal>> = vec![None; columns_count];
    let mut fixed_width = 0_i64;
    let mut fixed_count = 0_i64;
    let hundred = Decimal::from(100);
    for (index, column) in columns.iter().enumerate() {
        let mut char_width = -1_i64;
        let mut percent_width: Option<Decimal> = None;
        for (print_width, kind) in column {
            if print_width.is_empty() {
                continue;
            }
            let current: Decimal = print_width
                .trim()
                .parse()
                .map_err(|_| Error::new(format!("print-width {print_width} is not a number")))?;
            if current.is_zero() {
                char_width = 0;
                line += 1;
                continue;
            }
            if kind == "characters" {
                let whole = current.trunc().to_i64().unwrap_or(0);
                if whole > char_width {
                    char_width = whole;
                }
            } else if percent_width.is_none_or(|existing| current > existing) {
                percent_width = Some(current);
            }
        }
        if char_width >= 0 {
            match percent_width {
                Some(percent) => {
                    let percent_chars = (percent / hundred * Decimal::from(line)).trunc().to_i64().unwrap_or(0);
                    if percent_chars < char_width {
                        char_widths[index] = Some(char_width);
                        fixed_width += char_width;
                        fixed_count += 1;
                    } else {
                        percent_widths[index] = Some(percent);
                    }
                }
                None => {
                    char_widths[index] = Some(char_width);
                    fixed_width += char_width;
                    fixed_count += 1;
                }
            }
        } else if let Some(percent) = percent_width {
            percent_widths[index] = Some(percent);
        }
    }
    let for_percent = line - fixed_width;
    if for_percent < 0 {
        return Err(Error::new(format!(
            "fixed width columns exceeded total line characters {original} by {} characters",
            -for_percent
        )));
    }
    let percent_cols = i64::try_from(columns_count).unwrap_or(0) - fixed_count;
    let mut percent_total = Decimal::ZERO;
    for index in 0..columns_count {
        match percent_widths[index] {
            Some(percent) => percent_total += percent,
            None => {
                if char_widths[index].is_some() {
                    continue;
                }
                let share = (Decimal::ONE / Decimal::from(percent_cols.max(1)))
                    .round_dp_with_strategy(10, RoundingStrategy::MidpointAwayFromZero)
                    * hundred;
                percent_widths[index] = Some(share);
                percent_total += share;
            }
        }
    }
    let mut used = 0_i64;
    let scale = if percent_total.is_zero() {
        Decimal::ONE
    } else {
        (hundred / percent_total).round_dp_with_strategy(10, RoundingStrategy::MidpointAwayFromZero)
    };
    for index in 0..columns_count {
        let Some(percent) = percent_widths[index] else {
            continue;
        };
        let actual = percent * scale;
        percent_widths[index] = Some(actual);
        let chars = (actual / hundred * Decimal::from(for_percent))
            .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven)
            .to_i64()
            .unwrap_or(0);
        char_widths[index] = Some(chars);
        used += chars;
    }
    if used != for_percent && percent_cols > 0 {
        let mut remaining = for_percent - used;
        let per_column = (Decimal::from(remaining) / Decimal::from(percent_cols))
            .round_dp_with_strategy(0, RoundingStrategy::AwayFromZero)
            .to_i64()
            .unwrap_or(0);
        for index in 0..columns_count {
            if percent_widths[index].is_none() {
                continue;
            }
            let current = char_widths[index].unwrap_or(0);
            let adjust = if remaining.abs() > per_column.abs() {
                per_column
            } else {
                remaining
            };
            let new_chars = current + adjust;
            if new_chars > 0 {
                char_widths[index] = Some(new_chars);
                remaining -= adjust;
                if remaining == 0 {
                    break;
                }
            }
        }
    }
    Ok(char_widths.into_iter().map(|width| width.unwrap_or(0)).collect())
}

/// `sri.addNodeParameters`: the `parameter-map`, `parameter` children and `depends-on` values of an element.
///
/// # Errors
///
/// Returns an error when an expression fails.
pub fn node_parameters(parent: &NodeLit) -> Result<IndexMap<String, Value>, Error> {
    node_parameters_checked(parent).map(|(parameters, _)| parameters)
}

/// [`node_parameters`] and whether every `depends-on` field has a value in the context or the form map.
/// With `depends-optional` true an empty field only leaves its parameter out.
///
/// # Errors
///
/// Returns an error when an expression fails.
pub fn node_parameters_checked(parent: &NodeLit) -> Result<(IndexMap<String, Value>, bool), Error> {
    let mut parameters = IndexMap::new();
    let map_expr = parent.attr("parameter-map");
    if !map_expr.is_empty() {
        if let Some(map) = script::value_map(&script::eval_value(map_expr)?) {
            parameters.extend(map);
        }
    }
    for parameter in parent.children.iter().filter(|child| child.name == "parameter") {
        let name = parameter.attr("name");
        let from = if parameter.attr("from").is_empty() {
            name
        } else {
            parameter.attr("from")
        };
        parameters.insert(name.to_string(), context_value(from, parameter.attr("value"))?);
    }
    let optional = parent.attr("depends-optional") == "true";
    let mut has_all = true;
    for depends in parent.children.iter().filter(|child| child.name == "depends-on") {
        let field = depends.attr("field");
        let parameter = if depends.attr("parameter").is_empty() {
            field
        } else {
            depends.attr("parameter")
        };
        let mut value = script::context_value(field).unwrap_or(Value::Null);
        if is_empty(&value) {
            if let Some(found) = script::context_value("_formMap").and_then(|map| script::value_field(&map, field)) {
                value = found;
            }
        }
        if is_empty(&value) {
            has_all = false;
        } else {
            parameters.insert(parameter.to_string(), value);
        }
    }
    Ok((parameters, has_all || optional))
}

/// `sri.getContextValue(from, value)`.
///
/// # Errors
///
/// Returns an error when the expression fails.
pub fn context_value(from: &str, value: &str) -> Result<Value, Error> {
    if !value.is_empty() {
        return Ok(Value::String(script::expand_localized(value)?));
    }
    if !from.is_empty() {
        return script::eval_value(from);
    }
    Ok(Value::string(""))
}

/// `sri.setInContext(setNode)`: a `set` element run in the context.
///
/// # Errors
///
/// Returns an error when the expression fails.
pub fn set_in_context(set: &NodeLit) -> Result<(), Error> {
    let field = set.attr("field");
    if field.is_empty() {
        return Ok(());
    }
    let current = script::context_value(field);
    let skip = set.attr("set-if-empty") == "false" && current.as_ref().is_some_and(|value| !is_empty(value));
    if skip {
        return Ok(());
    }
    let mut value = if !set.attr("from").is_empty() {
        script::eval_value(set.attr("from"))?
    } else if !set.attr("value").is_empty() {
        Value::String(script::expand_localized(set.attr("value"))?)
    } else {
        Value::Null
    };
    if is_empty(&value) && !set.attr("default-value").is_empty() {
        value = Value::String(script::expand_localized(set.attr("default-value"))?);
    }
    if !set.attr("type").is_empty() {
        value = moqui_lib::object::basic_convert(&value, set.attr("type")).value;
    }
    script::set_context_name(field, value)
}

// ---- themes ----

/// `sri.getCurrentThemeId`: the user's theme for the screen theme type, else one of the user's groups', else the type's
/// `enumCode`, else a theme of the type with `DEFAULT` in its id. Found once for the render.
fn current_theme_id(state: &RenderState) -> String {
    if let Some(found) = state.theme_id.borrow().as_ref() {
        return found.clone();
    }
    let found = find_theme_id(state);
    *state.theme_id.borrow_mut() = Some(found.clone());
    found
}

fn find_theme_id(state: &RenderState) -> String {
    let mut theme_type = String::new();
    for def in state.url_info.render_defs() {
        if !def.screen_theme_type.is_empty() {
            theme_type.clone_from(&def.screen_theme_type);
        }
    }
    if theme_type.is_empty() {
        theme_type = "STT_INTERNAL".to_string();
    }
    let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() else {
        return String::new();
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let text_of = |row: Option<crate::entity::EntityValue>, field: &str| {
        row.and_then(|row| row.get(field).map(to_plain_string))
            .filter(|text| !text.is_empty())
    };
    let mut theme = None;
    if let Some(user_id) = user::current_user_id().ok().flatten() {
        theme = text_of(
            facade.find("moqui.security.UserScreenTheme").ok().and_then(|find| {
                find.eq("userId", Value::string(user_id))
                    .eq("screenThemeTypeEnumId", Value::string(&theme_type))
                    .one()
                    .ok()
                    .flatten()
            }),
            "screenThemeId",
        );
    }
    if theme.is_none() {
        // Reverse id order, so ALL_USERS comes after the named groups at the same sequence.
        let mut groups: Vec<String> = user::current_user_groups()
            .map(|groups| groups.iter().cloned().collect())
            .unwrap_or_default();
        groups.sort_unstable_by(|left, right| right.cmp(left));
        let rows = facade
            .find("moqui.security.UserGroupScreenTheme")
            .ok()
            .and_then(|find| {
                find.condition(moqui_db::sql::Condition::is_in(
                    "userGroupId",
                    groups.into_iter().map(Value::String).collect(),
                ))
                .eq("screenThemeTypeEnumId", Value::string(&theme_type))
                .order_by("sequenceNum", false)
                .order_by("userGroupId", true)
                .list()
                .ok()
            });
        theme = text_of(rows.and_then(|rows| rows.values().first().cloned()), "screenThemeId");
    }
    if theme.is_none() {
        theme = text_of(
            facade
                .find("moqui.basic.Enumeration")
                .ok()
                .and_then(|find| find.eq("enumId", Value::string(&theme_type)).one().ok().flatten()),
            "enumCode",
        );
    }
    if theme.is_none() {
        let rows = facade.find("moqui.screen.ScreenTheme").ok().and_then(|find| {
            find.eq("screenThemeTypeEnumId", Value::string(&theme_type))
                .condition(moqui_db::sql::Condition::like(
                    "screenThemeId",
                    Value::string("%DEFAULT%"),
                ))
                .list()
                .ok()
        });
        theme = text_of(rows.and_then(|rows| rows.values().first().cloned()), "screenThemeId");
    }
    theme.unwrap_or_default()
}

/// `sri.getThemeValues(type, themeId)`: the `ScreenThemeResource` values in sequence, kept for the render for the current theme.
///
/// # Errors
///
/// Returns an error when the entity cannot be read.
pub fn get_theme_values(state: &RenderState, resource_type: &str, theme_id: &str) -> Result<Vec<String>, Error> {
    let current = theme_id.is_empty() || theme_id == "null";
    if current {
        if let Some(found) = state.theme_values.borrow().get(resource_type) {
            return Ok(found.clone());
        }
    }
    let theme = if current {
        current_theme_id(state)
    } else {
        theme_id.to_string()
    };
    let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() else {
        return Ok(Vec::new());
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let list = facade
        .find("moqui.screen.ScreenThemeResource")?
        .eq("screenThemeId", Value::string(theme))
        .eq("resourceTypeEnumId", Value::string(resource_type))
        .order_by("sequenceNum", false)
        .list()?;
    let values: Vec<String> = list
        .iter()
        .filter_map(|row| row.get("resourceValue").map(to_plain_string))
        .filter(|text| !text.is_empty())
        .collect();
    if current {
        state
            .theme_values
            .borrow_mut()
            .insert(resource_type.to_string(), values.clone());
    }
    Ok(values)
}

/// `sri.getThemeIconClass(text)`: the icon class of the first `ScreenThemeIcon` pattern that matches, kept by text for the theme.
///
/// # Errors
///
/// Returns an error when the entity cannot be read.
pub fn get_theme_icon_class(state: &RenderState, text: &str) -> Result<Option<String>, Error> {
    let theme = current_theme_id(state);
    if let Some(found) = state.facade.theme_icon(&theme, text) {
        return Ok(found);
    }
    let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() else {
        return Ok(None);
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let list = facade
        .find("moqui.screen.ScreenThemeIcon")?
        .eq("screenThemeId", Value::string(&theme))
        .list()?;
    let mut icon = None;
    for row in list.iter() {
        let pattern = row.get("textPattern").map(to_plain_string).unwrap_or_default();
        if moqui_lib::object::regex_full_match(text, &pattern).unwrap_or(false) {
            icon = row.get("iconClass").map(to_plain_string);
            break;
        }
    }
    state.facade.put_theme_icon(&theme, text, icon.clone());
    Ok(icon)
}

// ---- html helpers ----

/// What the html macros read of a field's validation: input classes and a pattern with its message.
#[derive(Clone, Debug, Default)]
pub struct FieldValidation {
    pub classes: Vec<String>,
    pub regexp: Option<(String, String)>,
}

/// `FormInstance.getFieldValidationClasses` and `getFieldValidationRegexpInfo` for a field sub-node (`default-field`
/// and the like): `validate-service` and `validate-parameter`, else `validate-entity` and `validate-field`.
///
/// # Errors
///
/// Returns an error when the named entity does not exist.
pub fn field_validation(sub: &NodeLit) -> Result<FieldValidation, Error> {
    let field_name = sub.parent.as_ref().map_or("", |field| field.attr("name"));
    let mut out = FieldValidation::default();
    let service = sub.attr("validate-service");
    let entity = sub.attr("validate-entity");
    if !service.is_empty() {
        let parameter = match sub.attr("validate-parameter") {
            "" => field_name,
            other => other,
        };
        if let Some(hints) = crate::service::input_hints(service, parameter) {
            out.classes = hints.classes.iter().map(|class| (*class).to_string()).collect();
            out.regexp = hints.regexp;
        }
    } else if !entity.is_empty() {
        let name = match sub.attr("validate-field") {
            "" => field_name,
            other => other,
        };
        let shape = form::entity_shape(entity)?;
        if let Some((_, kind, _)) = shape.fields.iter().find(|(field, _, _)| field == name) {
            if kind.starts_with("number-") || kind.starts_with("currency-") {
                out.classes.push("number".to_string());
            }
        }
    }
    Ok(out)
}

/// `getFieldValidationJsRules` for a field sub-node: the browser checks of its `validate-service` parameter, or for a
/// numeric `validate-entity` field, each `(expression over value, localized message)`.
///
/// # Errors
///
/// Returns an error when the named entity does not exist.
pub fn field_validation_js_rules(sub: &NodeLit) -> Result<Vec<(String, String)>, Error> {
    let field_name = sub.parent.as_ref().map_or("", |field| field.attr("name"));
    let service = sub.attr("validate-service");
    let entity = sub.attr("validate-entity");
    let rules = if !service.is_empty() {
        let parameter = match sub.attr("validate-parameter") {
            "" => field_name,
            other => other,
        };
        crate::service::input_js_rules(service, parameter).unwrap_or_default()
    } else if !entity.is_empty() {
        let name = match sub.attr("validate-field") {
            "" => field_name,
            other => other,
        };
        let shape = form::entity_shape(entity)?;
        match shape.fields.iter().find(|(field, _, _)| field == name) {
            Some((_, kind, _)) if kind.starts_with("number-") || kind.starts_with("currency-") => {
                if kind.ends_with("integer") {
                    vec![(
                        crate::service::VALIDATE_NUMBER_INT.to_string(),
                        "Please enter a valid whole number".to_string(),
                    )]
                } else {
                    vec![(
                        crate::service::VALIDATE_NUMBER.to_string(),
                        "Please enter a valid number".to_string(),
                    )]
                }
            }
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    Ok(rules
        .into_iter()
        .map(|(expr, message)| (expr, l10n::localize(&message).unwrap_or(message)))
        .collect())
}

/// `sri.getFieldTransitionValue(transition, node, term, labelField, alwaysGet)`: the label a transition gives for a
/// value, as the widget's options would show it, by a test render of the transition with `term` and the node's
/// parameters. None with no term (unless `alwaysGet`) or while a `depends-on` field is empty.
///
/// # Errors
///
/// Returns an error when the transition cannot be rendered.
pub fn field_transition_value(
    state: &RenderState,
    transition: &str,
    node: &NodeLit,
    term: &str,
    label_field: &str,
    always_get: bool,
) -> Result<Option<String>, Error> {
    if !always_get && term.is_empty() {
        return Ok(None);
    }
    let label_field = if label_field.is_empty() { "label" } else { label_field };
    let (mut parameters, has_all_depends) = node_parameters_checked(node)?;
    if !has_all_depends {
        return Ok(None);
    }
    parameters.insert("term".to_string(), Value::string(term));
    let url = super::urls::build_url(state, transition)?;
    let root = state
        .url_info
        .path_defs
        .first()
        .map(|def| def.location.clone())
        .ok_or_else(|| Error::new("no root screen"))?;
    let parameters: std::collections::BTreeMap<String, Value> = parameters.into_iter().collect();
    let rendered = super::screen_test::ScreenTest::new(&state.factory)
        .root_screen(root)
        .render(&url.path_with_params(), parameters)?;
    let first_label = |item: &Value| match item {
        Value::Map(entry) => entry.get(label_field).map(to_plain_string),
        other => Some(to_plain_string(other)),
    };
    Ok(match rendered.json_value() {
        Ok(Value::List(items)) => items.first().and_then(first_label),
        Ok(Value::Map(entries)) => match entries.get("options") {
            Some(Value::List(items)) if !items.is_empty() => items.first().and_then(first_label),
            _ => entries.get(label_field).map(to_plain_string),
        },
        Ok(Value::Null) => None,
        Ok(other) => Some(to_plain_string(&other)),
        Err(_) => Some(rendered.text()),
    })
}

/// One item of a screen's menu: a subscreen with the URL of its minimal path.
#[derive(Clone, Debug, Default)]
pub struct MenuEntry {
    pub name: String,
    /// The subscreen takes parameters, so a list of applications leaves it out.
    pub has_parameters: bool,
    pub title: String,
    pub url: String,
    pub disabled: bool,
    pub in_path: bool,
    pub image: String,
    pub image_type: String,
}

/// `getMenuSubscreensItems` of the active screen with what the macros ask of each item's URL: the
/// subscreens that appear in the menu, in menu order, that the user may view.
///
/// # Errors
///
/// Returns an error when a path cannot be resolved or a `disable-when` expression fails.
pub fn menu_items(state: &RenderState, screen_path: Option<&str>) -> Result<Vec<MenuEntry>, Error> {
    let root = state
        .url_info
        .path_defs
        .first()
        .map(|root| root.location.clone())
        .ok_or_else(|| Error::new("no root screen"))?;
    // The items of another screen: the path is resolved from the root, and its items are paths below it.
    let (def, prefix) = match screen_path {
        Some(path) => {
            let names = super::url::resolve_screen_path(&[], path);
            let info = ScreenUrlInfo::resolve(&state.factory, &root, &names, 0)?;
            // The screen the path names, not the default subscreen the walk goes on to.
            let named = info.path_defs.get(names.len()).unwrap_or(&info.target_screen);
            (Arc::clone(named), Some(names))
        }
        None => (active_def(state), None),
    };
    let mut out = Vec::new();
    for item in def
        .subscreens
        .sorted()
        .into_iter()
        .filter(|item| item.menu_include && item.is_valid_in_current_context())
    {
        let url = match &prefix {
            Some(names) => {
                let mut full = names.clone();
                full.push(item.name.clone());
                super::urls::build_url_names(state, full)?
            }
            None => super::urls::build_url(state, &item.name)?,
        };
        if !url.exists {
            continue;
        }
        let Ok(target) = state.facade.screen_def(&state.factory, &item.location) else {
            continue;
        };
        if !screen_permitted(&target) {
            continue;
        }
        let disabled = match &item.disable_when {
            Some(expression) => script::eval_condition(expression)?,
            None => false,
        };
        let image = ScreenUrlInfo::resolve(&state.factory, &root, &url.names, 0)
            .ok()
            .and_then(|info| info.menu_image)
            .unwrap_or_default();
        out.push(MenuEntry {
            name: item.name.clone(),
            has_parameters: !target.parameters.is_empty(),
            title: item.menu_title.clone(),
            url: url.minimal_path_url_with_params(),
            disabled,
            in_path: state.url_info.full_path.starts_with(&url.minimal_names),
            image: image.0,
            image_type: image.1,
        });
    }
    Ok(out)
}

/// Whether the user may view a screen: a screen that requires authentication needs a user and
/// an authorization record.
pub(super) fn screen_permitted(def: &ScreenDef) -> bool {
    let required = def.authentication == super::definition::Authentication::Required;
    if required && user::current_username().ok().flatten().is_none() {
        return false;
    }
    ExecutionContext::with_current(|ec| {
        match ec.push_checked("AT_XML_SCREEN", &def.location, "view", required, false, None) {
            Ok(()) => {
                // A check, not an access: Java's isPermitted, which is not counted as a hit.
                ec.untrack_hit();
                let _ = ec.pop();
                true
            }
            Err(_) => false,
        }
    })
    .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::{encode_html, form_list_column_char_widths};

    fn columns(specs: &[(&str, &str)]) -> Vec<Vec<(String, String)>> {
        specs
            .iter()
            .map(|(width, kind)| vec![((*width).to_string(), (*kind).to_string())])
            .collect()
    }

    #[test]
    fn equal_columns_share_the_line_and_the_remainder_goes_to_the_first() {
        // 132 - 2 separators = 130; a third of it is 43.33
        assert_eq!(
            form_list_column_char_widths(&columns(&[("", ""), ("", ""), ("", "")]), 132).unwrap(),
            [44, 43, 43]
        );
        // 5 columns: 128 shared, 25.6 each rounds to 26 and the 2 over come back from the first two
        assert_eq!(
            form_list_column_char_widths(&columns(&[("", ""); 5]), 132).unwrap(),
            [25, 25, 26, 26, 26]
        );
        // a line width of 0 is 132
        assert_eq!(form_list_column_char_widths(&columns(&[("", "")]), 0).unwrap(), [132]);
        assert!(form_list_column_char_widths(&[], 80).unwrap().is_empty());
    }

    #[test]
    fn fixed_percent_and_zero_widths() {
        // a fixed column of 20 characters, the other two split the 110 left
        assert_eq!(
            form_list_column_char_widths(&columns(&[("20", "characters"), ("", ""), ("", "")]), 132).unwrap(),
            [20, 55, 55]
        );
        // a percent width is scaled with the others to 100 percent
        assert_eq!(
            form_list_column_char_widths(&columns(&[("50", ""), ("", ""), ("", "")]), 132).unwrap(),
            [56, 37, 37]
        );
        // a zero width hides the column and gives its separator back
        assert_eq!(
            form_list_column_char_widths(&columns(&[("0", ""), ("", ""), ("", "")]), 132).unwrap(),
            [0, 65, 66]
        );
        // a percent that is fewer characters than the fixed width gives way to it
        assert_eq!(
            form_list_column_char_widths(&columns(&[("40", "characters"), ("10", "")]), 132).unwrap(),
            [40, 91]
        );
        let err =
            form_list_column_char_widths(&columns(&[("100", "characters"), ("100", "characters")]), 132).unwrap_err();
        assert!(err.message().contains("exceeded total line characters 132 by"), "{err}");
        assert!(form_list_column_char_widths(&columns(&[("x", "")]), 80).is_err());
    }

    #[test]
    fn html_is_encoded_for_text_the_page_shows() {
        assert_eq!(
            encode_html("<a href=\"x\">T&C's</a>"),
            "&lt;a href=&quot;x&quot;&gt;T&amp;C&#39;s&lt;/a&gt;"
        );
    }
}
