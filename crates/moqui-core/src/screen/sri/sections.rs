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

//! Render control: the active screen, sections, section-iterate, subscreens, included screens and text.

use std::sync::Arc;

use indexmap::IndexMap;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use super::{active_def, encode_html, write, NodeLit, Runner};
use crate::context::{Error, ErrorKind, ExecutionContext};
use crate::screen::definition::{ScreenDef, SectionDef, SectionKind};
use crate::screen::generate::UnitRequest;
use crate::screen::render::{check_condition, Frames, RenderState};
use crate::script::ContextFrame;
use crate::{script, web};

fn is_security(err: &Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::AuthenticationRequired | ErrorKind::Unauthorized | ErrorKind::Tarpit
    )
}

/// Run a nested render. As in Java, an error other than a security one is logged and its message is
/// written as text where the output would have been, so one section does not lose the page. In production the text
/// names only an error id; the detail is in the log under it.
pub(super) fn guarded(state: &RenderState, label: &str, body: impl FnOnce() -> Result<(), Error>) -> Result<(), Error> {
    match body() {
        Err(err) if !is_security(&err) => {
            let detail = format!("Error rendering {label}: {err}");
            let text = if web::is_production() {
                format!(
                    "Error rendering {label}: internal error, id {}",
                    web::log_error_id(&detail)
                )
            } else {
                tracing::error!("{detail}");
                format!("Error rendering {label}: {}", err.message())
            };
            write(state, &encode_html(&text));
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
            let mut items = iteration_entries(&value);
            let _frame = paginate.then(ContextFrame::push);
            if *paginate {
                items = paginate_items(list, items)?;
            }
            iterate_section(state, runner, def, section, name, entry, key.as_deref(), &items)
        }
        _ => render_single(state, runner, def, section, name),
    }
}

/// The entries of a section-iterate list: each item, and for a map its key and value.
pub(in crate::screen) fn iteration_entries(value: &Value) -> Vec<(Option<String>, Value)> {
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
/// the list's count and page variables put in the context. A list that is already a page (`{list}AlreadyPaginated`,
/// or a `{list}Count` from a paginated find) is returned as it is.
pub(in crate::screen) fn paginate_items(
    list_name: &str,
    items: Vec<(Option<String>, Value)>,
) -> Result<Vec<(Option<String>, Value)>, Error> {
    let mut context = script::context_combined();
    let already = format!("{list_name}AlreadyPaginated");
    if context.contains_key(&already) {
        return Ok(items);
    }
    let size = i32::try_from(items.len()).map_err(|_| Error::new("list is too long to paginate"))?;
    moqui_lib::collection::paginate_parameters(size, list_name, &mut context)?;
    if context.contains_key(&already) {
        return Ok(items);
    }
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
        let frame = ContextFrame::push();
        let _ = script::set_context_name(entry, item.clone());
        if let (Some(key_name), Some(item_key)) = (key, item_key) {
            let _ = script::set_context_name(key_name, Value::string(item_key));
        }
        let _ = script::set_context_name("sectionEntryIndex", Value::I64(index as i64));
        let _ = script::set_context_name(&format!("{entry}_index"), Value::I64(index as i64));
        let _ = script::set_context_name(&format!("{entry}_has_next"), Value::Bool(index + 1 < items.len()));
        let result = render_single(state, runner, def, section, name);
        drop(frame);
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
            actions.run()?;
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
    let _frame = (!share_scope).then(ContextFrame::push);
    let previous = state.override_def.borrow().clone();
    let result = guarded(state, &format!("screen [{location}]"), || {
        let def = state.facade.screen_def(&state.factory, location)?;
        *state.override_def.borrow_mut() = Some(Arc::clone(&def));
        render_def(state, runner, &def, false)
    });
    *state.override_def.borrow_mut() = previous;
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
    let rendered = {
        let _frame = ContextFrame::push();
        registry.render_location_default(&state.factory, location, None, template)
    };
    write(state, &rendered?);
    Ok(String::new())
}

pub fn append_to_after_screen_writer(state: &RenderState, text: &str) {
    state.after_text.borrow_mut().push_str(text);
}

pub fn append_to_script_writer(state: &RenderState, text: &str) {
    state.script_text.borrow_mut().push_str(text);
}
