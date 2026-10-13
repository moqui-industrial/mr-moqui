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

//! Screen render code: the macro modules for a render mode and the generated function for a section.
//!
//! A section's widgets (or fail-widgets) are walked once per render mode. The result is a
//! JavaScript function that runs for each request on the server runtime, calls `sri` and `ec`, and
//! writes through `sri.write`. It is kept with the screen definition, so it lives until the screen
//! file changes. Set `MOQUI_ACTIONS_DEBUG` to put a comment naming the element before each statement.
//!
//! The macro modules are embedded in the binary. A `macro-template-location` that is not the
//! built-in path and names a file that exists replaces the embedded copy of that mode, which is
//! how a theme overrides a mode. `ScreenSupport.js` is never replaced.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use moqui_lib::node::MNode;
use rquickjs::{Array, Ctx, Object};

use super::cache::ScreenFacade;
use super::definition::ScreenDef;
use super::form::{self, FormInstance};
use crate::context::{Error, ExecutionContextFactory};
use crate::script::walker::{self, Resolve, Walk};

const TEMPLATE_SUPPORT: &str = include_str!("../../../../framework/template/TemplateSupport.js");
const SCREEN_SUPPORT: &str = include_str!("../../../../framework/template/screen/ScreenSupport.js");

/// The mode modules built into the binary, by file name.
const MODE_MODULES: [(&str, &str); 4] = [
    (
        "ScreenText.js",
        include_str!("../../../../framework/template/screen/ScreenText.js"),
    ),
    (
        "ScreenCsv.js",
        include_str!("../../../../framework/template/screen/ScreenCsv.js"),
    ),
    (
        "ScreenXml.js",
        include_str!("../../../../framework/template/screen/ScreenXml.js"),
    ),
    (
        "ScreenHtmlPrint.js",
        include_str!("../../../../framework/template/screen/ScreenHtmlPrint.js"),
    ),
];

const BUILT_IN_DIR: &str = "framework/template/screen/";
const DEBUG_ENV: &str = "MOQUI_ACTIONS_DEBUG";

static NEXT_UNIT: AtomicU64 = AtomicU64::new(1);

/// The generated function for one section in one render mode.
#[derive(Debug)]
pub struct Unit {
    /// Distinguishes this generation from any other, so a runtime can keep the compiled function.
    pub id: u64,
    /// A function expression: the mode's runtime prelude and the body.
    pub source: String,
}

/// What a definition keeps for rendering: generated units and assembled forms.
#[derive(Default)]
pub struct Generated {
    units: Mutex<HashMap<String, Option<Arc<Unit>>>>,
    forms: Mutex<HashMap<String, Arc<FormInstance>>>,
}

impl std::fmt::Debug for Generated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Generated")
    }
}

impl Generated {
    pub(super) fn form(&self, name: &str) -> Option<Arc<FormInstance>> {
        self.forms
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(name)
            .cloned()
    }

    pub(super) fn put_form(&self, name: &str, form: Arc<FormInstance>) {
        self.forms
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(name.to_string(), form);
    }

    fn unit(&self, key: &str) -> Option<Option<Arc<Unit>>> {
        self.units
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(key)
            .cloned()
    }

    fn put_unit(&self, key: String, unit: Option<Arc<Unit>>) {
        self.units
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(key, unit);
    }

    /// The number of units generated, for tests.
    #[cfg(test)]
    pub fn unit_count(&self) -> usize {
        self.units.lock().unwrap_or_else(|err| err.into_inner()).len()
    }
}

/// Which section's code to make.
pub struct UnitRequest<'a> {
    pub factory: &'a ExecutionContextFactory,
    pub facade: &'a ScreenFacade,
    pub def: &'a Arc<ScreenDef>,
    /// The section name. Empty is the screen's own root section.
    pub section: &'a str,
    pub fail: bool,
    pub mode: &'a str,
    pub macro_location: &'a str,
}

/// The unit for a section, from the definition's cache when it was made before. None when the
/// section has no widgets (or no fail-widgets).
///
/// # Errors
///
/// Returns an error when the module cannot be loaded, an element has no macro, a macro throws, or
/// the generated code does not parse.
pub fn unit_for(ctx: &Ctx<'_>, request: &UnitRequest<'_>) -> Result<Option<Arc<Unit>>, Error> {
    let key = format!(
        "{}|{}|{}|{}",
        request.mode,
        request.macro_location,
        request.section,
        if request.fail { "fail" } else { "widgets" }
    );
    if let Some(found) = request.def.generated.unit(&key) {
        return Ok(found);
    }
    let made = match unit_root(request.def, request.section, request.fail)? {
        Some(root) => match mode_source(request.factory, request.macro_location) {
            Ok(source) => {
                let loaded = load_mode_source(ctx, &source, request.mode, std::env::var_os(DEBUG_ENV).is_some())?;
                let resolve = |node: &MNode| -> Result<Option<MNode>, Error> {
                    let name = node.name();
                    if name != "form-single" && name != "form-list" {
                        return Ok(None);
                    }
                    let form_name = node.attribute("name").unwrap_or_default();
                    if request.def.forms.get(&form_name).is_some_and(|form| form.dynamic) {
                        // Its fields depend on the context, so the call that renders it assembles it then.
                        let call = MNode::new("render-dynamic-form");
                        call.set_attribute("name", form_name);
                        return Ok(Some(call));
                    }
                    let assembled = form::assemble(request.facade, request.factory, request.def, &form_name)?;
                    assembled.node().map(Some)
                };
                let body = generate_body(ctx, &loaded, &root, Some(&resolve), request.mode).map_err(|err| {
                    Error::new(format!(
                        "{} {}: {}",
                        request.def.location,
                        describe(request),
                        err.message()
                    ))
                })?;
                Some(Arc::new(make_unit(&loaded.prelude, body)?))
            }
            // A mode with no module yet renders nothing for widgets with nothing in them.
            Err(_) if is_empty_root(&root) => None,
            Err(err) => {
                return Err(Error::new(format!(
                    "{} {}: {}",
                    request.def.location,
                    describe(request),
                    err.message()
                )))
            }
        },
        None => None,
    };
    request.def.generated.put_unit(key, made.clone());
    Ok(made)
}

/// The unit for a dynamic form as it was just assembled. The code is kept under the text of the
/// assembled form, so a form that comes out the same each time is generated once.
///
/// # Errors
///
/// Returns an error when the module cannot be loaded or the form's code cannot be generated.
pub fn unit_for_dynamic_form(
    ctx: &Ctx<'_>,
    request: &UnitRequest<'_>,
    form: &FormInstance,
) -> Result<Arc<Unit>, Error> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    form.xml.hash(&mut hasher);
    let key = format!(
        "dynamic|{}|{}|{}|{:x}",
        request.mode,
        request.macro_location,
        form.name,
        hasher.finish()
    );
    if let Some(Some(found)) = request.def.generated.unit(&key) {
        return Ok(found);
    }
    let source = mode_source(request.factory, request.macro_location)?;
    let loaded = load_mode_source(ctx, &source, request.mode, std::env::var_os(DEBUG_ENV).is_some())?;
    let root = MNode::new("dynamic-form");
    root.append_node(form.node()?);
    let body = generate_body(ctx, &loaded, &root, None, request.mode).map_err(|err| {
        Error::new(format!(
            "{} dynamic form {}: {}",
            request.def.location,
            form.name,
            err.message()
        ))
    })?;
    let unit = Arc::new(make_unit(&loaded.prelude, body)?);
    request.def.generated.put_unit(key, Some(Arc::clone(&unit)));
    Ok(unit)
}

fn is_empty_root(root: &MNode) -> bool {
    match root.first_named("widgets") {
        Some(widgets) => widgets.children().is_empty(),
        None => root.children().is_empty(),
    }
}

fn describe(request: &UnitRequest<'_>) -> String {
    let which = if request.fail { "fail-widgets" } else { "widgets" };
    if request.section.is_empty() {
        format!("{which} ({})", request.mode)
    } else {
        format!("section {} {which} ({})", request.section, request.mode)
    }
}

fn make_unit(prelude: &str, body: String) -> Result<Unit, Error> {
    let source = format!("(function () {{\n\"use strict\";\n{prelude}{body}}})");
    crate::script::check_program(&source).map_err(|err| Error::new(format!("{}\n{source}", err.message())))?;
    Ok(Unit {
        id: NEXT_UNIT.fetch_add(1, Ordering::Relaxed),
        source,
    })
}

/// The tree to walk for a section: the section's widgets, or for the root section a `screen`
/// element around them, which the xml mode reads to name its root element.
fn unit_root(def: &ScreenDef, section: &str, fail: bool) -> Result<Option<MNode>, Error> {
    let fragment = if section.is_empty() {
        if fail {
            def.root.fail_widgets.as_ref()
        } else {
            def.root.widgets.as_ref()
        }
    } else {
        let found = def
            .sections
            .get(section)
            .ok_or_else(|| Error::new(format!("No section with name [{section}] in screen [{}]", def.location)))?;
        if fail {
            found.fail_widgets.as_ref()
        } else {
            found.widgets.as_ref()
        }
    };
    let Some(fragment) = fragment else {
        return Ok(None);
    };
    let widgets = fragment.node()?;
    if section.is_empty() && !fail {
        let screen = MNode::new("screen");
        screen.append_node(widgets);
        Ok(Some(screen))
    } else {
        Ok(Some(widgets))
    }
}

/// A loaded mode: the macro table, the elements it treats as opaque, and its runtime prelude.
pub struct ModeModule<'js> {
    macros: Object<'js>,
    opaque: HashSet<String>,
    prelude: String,
}

fn mode_source(factory: &ExecutionContextFactory, location: &str) -> Result<String, Error> {
    let file = location.rsplit('/').next().unwrap_or(location);
    let built_in = format!("{BUILT_IN_DIR}{file}");
    if location != built_in {
        if let Ok(resource) = factory.resource().get(location) {
            if resource.exists() && resource.is_file() {
                return resource.read_text();
            }
        }
    }
    MODE_MODULES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, source)| (*source).to_string())
        .ok_or_else(|| {
            Error::new(format!(
                "there is no macro module {file} at {location}; html, qvt, qjs and qvue come in a later slice"
            ))
        })
}

fn load_mode_source<'js>(ctx: &Ctx<'js>, source: &str, mode: &str, debug: bool) -> Result<ModeModule<'js>, Error> {
    let chain = walker::load_chain(ctx, TEMPLATE_SUPPORT, &[SCREEN_SUPPORT, source], debug)?;
    let module = chain
        .into_iter()
        .next_back()
        .ok_or_else(|| Error::new(format!("no macro module for {mode}")))?;
    let macros: Object = module
        .get("macros")
        .map_err(|_| Error::new(format!("macro module for {mode} has no macros")))?;
    let opaque: Array = module
        .get("opaque")
        .map_err(|_| Error::new(format!("macro module for {mode} has no opaque list")))?;
    let prelude: String = module.get("prelude").unwrap_or_default();
    Ok(ModeModule {
        macros,
        opaque: walker::names_of(&opaque)?,
        prelude,
    })
}

fn generate_body<'js>(
    ctx: &Ctx<'js>,
    module: &ModeModule<'js>,
    root: &MNode,
    resolve: Option<Resolve<'_>>,
    mode: &str,
) -> Result<String, Error> {
    let missing = format!("screen macro in {mode}, element");
    let walk = Walk {
        macros: &module.macros,
        opaque: &module.opaque,
        resolve,
        missing: &missing,
    };
    let (source, _) = walker::walk(ctx, &walk, root, None)?;
    Ok(source.replace(['\u{1}', '\u{2}'], ""))
}

/// Generate the body for a tree of widgets with the built-in module for `mode`, forms as given. For
/// tests: it reads no screen file and caches nothing.
///
/// # Errors
///
/// Returns an error when the XML does not parse or generate.
#[cfg(test)]
pub fn generate_xml(mode: &str, xml: &str, debug_comments: bool) -> Result<String, Error> {
    let file = match mode {
        "text" => "ScreenText.js",
        "csv" => "ScreenCsv.js",
        "xml" => "ScreenXml.js",
        "html-print" => "ScreenHtmlPrint.js",
        other => return Err(Error::new(format!("no built-in macro module for render mode {other}"))),
    };
    let source = MODE_MODULES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, source)| *source)
        .ok_or_else(|| Error::new("missing built-in module"))?;
    let root = MNode::parse_text("fixture", xml)?;
    crate::script::walker_context(|ctx| {
        let module = load_mode_source(ctx, source, mode, debug_comments)?;
        generate_body(ctx, &module, &root, None, mode)
    })
}

/// The complete function source for a tree of widgets, as a unit would run it. For tests.
///
/// # Errors
///
/// Returns an error when the XML does not parse or generate.
#[cfg(test)]
pub fn generate_unit_source(mode: &str, xml: &str) -> Result<String, Error> {
    let file = match mode {
        "text" => "ScreenText.js",
        "csv" => "ScreenCsv.js",
        "html-print" => "ScreenHtmlPrint.js",
        _ => "ScreenXml.js",
    };
    let source = MODE_MODULES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, source)| *source)
        .ok_or_else(|| Error::new("missing built-in module"))?;
    let root = MNode::parse_text("fixture", xml)?;
    crate::script::walker_context(|ctx| {
        let module = load_mode_source(ctx, source, mode, false)?;
        let body = generate_body(ctx, &module, &root, None, mode)?;
        Ok(make_unit(&module.prelude, body)?.source)
    })
}
