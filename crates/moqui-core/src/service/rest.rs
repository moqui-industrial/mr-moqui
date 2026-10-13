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

//! REST API. `*.rest.xml` is walked here. There is no HTTP route. JSON-RPC is not included.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use indexmap::IndexMap;

use moqui_lib::node::MNode;
use moqui_lib::object::{is_empty, Value};

use super::call;
use crate::context::{Error, ErrorKind, ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;

/// The REST resources of a factory, read once from its components' `service/*.rest.xml` files.
struct Roots {
    resources: BTreeMap<String, Node>,
}

type Shared = RwLock<Arc<Roots>>;

/// Walk `path` through `*.rest.xml` and run the `<entity>` or `<service>` operation.
///
/// The first element is the root resource `name`. `method` is the HTTP method, in any case, and
/// comes from the request, never from the parameters. The errors have kinds the web layer maps to
/// status codes: not found, method not allowed, and authentication required.
///
/// # Errors
///
/// Returns an error when the path or method is not served, or the operation fails.
pub fn run(path: &[&str], method: &str, parameters: &BTreeMap<String, Value>) -> Result<Value, Error> {
    if path.is_empty() {
        return Err(Error::of_kind(
            ErrorKind::NotFound,
            "Cannot run REST service with no path",
        ));
    }
    let method = method.trim().to_ascii_lowercase();
    let mut params = parameters.clone();
    let roots = current_roots()?;
    let root = roots.resources.get(path[0]).ok_or_else(|| {
        Error::of_kind(
            ErrorKind::NotFound,
            format!("Service REST API root resource not found with name {}", path[0]),
        )
    })?;
    root.visit(path, 0, &method, &mut params)
}

fn current_roots() -> Result<Arc<Roots>, Error> {
    let factory = ExecutionContext::with_current(|ctx| Arc::clone(ctx.factory()))
        .ok_or_else(|| Error::new("no execution context is entered"))?;
    Ok(roots_of(&factory))
}

fn roots_of(factory: &Arc<ExecutionContextFactory>) -> Arc<Roots> {
    let shared = factory.extension::<Shared>(|factory| RwLock::new(Arc::new(load(factory))));
    let guard = shared.read().unwrap_or_else(|err| err.into_inner());
    Arc::clone(&guard)
}

/// Read every component's REST files. A file that does not load is logged and left out, so one bad
/// file never takes the other resources down. A later file with the same root name replaces an earlier one.
fn load(factory: &ExecutionContextFactory) -> Roots {
    let mut resources = BTreeMap::new();
    for component in factory.components() {
        let dir = component.location.join("service");
        load_dir(&mut resources, &dir);
    }
    tracing::info!(roots = resources.len(), "loaded REST API resources");
    Roots { resources }
}

fn load_dir(resources: &mut BTreeMap<String, Node>, dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".rest.xml"))
        })
        .collect();
    paths.sort();
    for path in paths {
        if let Err(err) = insert_file(resources, &path) {
            tracing::error!(file = %path.display(), error = %err, "REST file did not load");
        }
    }
}

fn insert_file(roots: &mut BTreeMap<String, Node>, path: &Path) -> Result<(), Error> {
    let root = MNode::parse_file(path)?;
    if root.name() != "resource" {
        return Err(Error::new(format!(
            "{} root is {} and not resource",
            path.display(),
            root.name()
        )));
    }
    let node = node_from(&root, "true", false)?;
    if roots.contains_key(&node.name) {
        tracing::warn!(resource = %node.name, file = %path.display(), "REST root resource replaced by a later file");
    }
    roots.insert(node.name.clone(), node);
    Ok(())
}

/// Replace a factory's resources with those in `paths`, for tests that bring their own files.
#[cfg(test)]
pub(crate) fn install(factory: &Arc<ExecutionContextFactory>, paths: &[PathBuf]) {
    let mut resources = BTreeMap::new();
    for path in paths {
        insert_file(&mut resources, path).expect("rest file");
    }
    let shared = factory.extension::<Shared>(|_| {
        RwLock::new(Arc::new(Roots {
            resources: BTreeMap::new(),
        }))
    });
    *shared.write().unwrap_or_else(|err| err.into_inner()) = Arc::new(Roots { resources });
}

struct Node {
    name: String,
    is_id: bool,
    allow_extra_path: bool,
    methods: BTreeMap<String, Operation>,
    resources: BTreeMap<String, Node>,
    id: Option<Box<Node>>,
}

enum Operation {
    Service {
        name: String,
        require_authentication: String,
    },
    Entity {
        name: String,
        master_name: Option<String>,
        operation: String,
        require_authentication: String,
    },
}

fn node_from(node: &MNode, parent_auth: &str, is_id: bool) -> Result<Node, Error> {
    let name = node.attribute("name").unwrap_or_default();
    if name.is_empty() {
        return Err(Error::new("REST resource is missing a name"));
    }
    let auth = node
        .attribute("require-authentication")
        .unwrap_or_else(|| parent_auth.to_string());
    let mut methods = BTreeMap::new();
    let mut resources = BTreeMap::new();
    let mut id = None;
    for child in node.children() {
        match child.name().as_str() {
            "method" => {
                let verb = child.attribute("type").unwrap_or_default().to_ascii_lowercase();
                if verb.is_empty() {
                    return Err(Error::new(format!("REST method on {name} is missing a type")));
                }
                methods.insert(verb, operation_from(&child, &auth)?);
            }
            "resource" => {
                let child_node = node_from(&child, &auth, false)?;
                resources.insert(child_node.name.clone(), child_node);
            }
            "id" => id = Some(Box::new(node_from(&child, &auth, true)?)),
            _ => {}
        }
    }
    Ok(Node {
        name,
        is_id,
        allow_extra_path: is_id && node.attribute("allow-extra-path").as_deref() == Some("true"),
        methods,
        resources,
        id,
    })
}

fn operation_from(method: &MNode, parent_auth: &str) -> Result<Operation, Error> {
    let auth = method
        .attribute("require-authentication")
        .unwrap_or_else(|| parent_auth.to_string());
    let body = method
        .children()
        .into_iter()
        .next()
        .ok_or_else(|| Error::new("REST method has no operation"))?;
    match body.name().as_str() {
        "service" => {
            let name = body.attribute("name").unwrap_or_default();
            if name.is_empty() {
                return Err(Error::new("REST service operation is missing a name"));
            }
            Ok(Operation::Service {
                name,
                require_authentication: auth,
            })
        }
        "entity" => {
            let name = body.attribute("name").unwrap_or_default();
            let operation = body.attribute("operation").unwrap_or_default();
            if name.is_empty() || operation.is_empty() {
                return Err(Error::new("REST entity operation is missing a name or operation"));
            }
            Ok(Operation::Entity {
                name,
                master_name: body.attribute("masterName").filter(|text| !text.is_empty()),
                operation,
                require_authentication: auth,
            })
        }
        other => Err(Error::new(format!("REST method operation {other} is not supported"))),
    }
}

impl Node {
    fn visit(
        &self,
        path: &[&str],
        index: usize,
        method: &str,
        params: &mut BTreeMap<String, Value>,
    ) -> Result<Value, Error> {
        let index = if self.is_id {
            self.bind_id(path, index, params);
            if self.allow_extra_path {
                path.len().saturating_sub(1)
            } else {
                index
            }
        } else {
            index
        };
        self.finish(path, index, method, params)
    }

    fn bind_id(&self, path: &[&str], index: usize, params: &mut BTreeMap<String, Value>) {
        if self.allow_extra_path {
            let items = path
                .get(index..)
                .unwrap_or(&[])
                .iter()
                .copied()
                .map(Value::string)
                .collect();
            params.insert(self.name.clone(), Value::List(items));
            return;
        }
        let value = path.get(index).copied().unwrap_or("");
        params.insert(self.name.clone(), Value::string(value));
    }

    fn finish(
        &self,
        path: &[&str],
        index: usize,
        method: &str,
        params: &mut BTreeMap<String, Value>,
    ) -> Result<Value, Error> {
        let next = index + 1;
        if path.len() > next {
            let segment = path[next];
            if let Some(child) = self.resources.get(segment) {
                return child.visit(path, next, method, params);
            }
            if let Some(id) = &self.id {
                return id.visit(path, next, method, params);
            }
            let names = self.resources.keys().map(String::as_str).collect::<Vec<_>>().join(", ");
            return Err(Error::of_kind(
                ErrorKind::NotFound,
                format!(
                    "Resource {segment} not valid, index {index} in path {}; resources available are {names}",
                    path.join("/")
                ),
            ));
        }
        if !self.methods.contains_key(method) {
            if let Some(id) = &self.id {
                if id.allow_extra_path {
                    return id.visit(path, next, method, params);
                }
            }
            return Err(Error::of_kind(
                ErrorKind::MethodNotAllowed,
                format!("Method {method} not supported at {}", path.join("/")),
            ));
        }
        let operation = &self.methods[method];
        match operation {
            Operation::Service {
                name,
                require_authentication,
            } => run_service(name, require_authentication, params),
            Operation::Entity {
                name,
                master_name,
                operation,
                require_authentication,
            } => run_entity(name, master_name.as_deref(), operation, require_authentication, params),
        }
    }
}

fn run_service(name: &str, auth: &str, params: &BTreeMap<String, Value>) -> Result<Value, Error> {
    require_user(auth, &format!("call service {name}"))?;
    Ok(value_map(call::sync(name, params)?))
}

fn run_entity(
    name: &str,
    master_name: Option<&str>,
    operation: &str,
    auth: &str,
    params: &BTreeMap<String, Value>,
) -> Result<Value, Error> {
    require_user(auth, &format!("operation {operation} on entity {name}"))?;
    let facade = current_facade()?;
    match operation {
        "list" | "one" => find_entity(&facade, name, master_name, operation, params),
        "count" => count_entity(&facade, name, params),
        "create" | "update" | "store" | "delete" => {
            let service = format!("{operation}#{name}");
            Ok(value_map(call::sync(&service, params)?))
        }
        other => Err(Error::new(format!(
            "Entity operation {other} not supported, must be one of: one, list, count, create, update, store, delete"
        ))),
    }
}

fn find_entity(
    facade: &EntityFacade,
    name: &str,
    master_name: Option<&str>,
    operation: &str,
    params: &BTreeMap<String, Value>,
) -> Result<Value, Error> {
    let mut parms = params.clone();
    if let Some(master) = master_name {
        parms
            .entry("master".to_string())
            .or_insert_with(|| Value::string(master));
    }
    let found = match facade.rest("find", name, &parms) {
        Ok(value) => value,
        Err(err) if operation == "one" && err.message().contains("No value found") => return Ok(Value::Null),
        Err(err) => return Err(err.into()),
    };
    if operation == "list" {
        return Ok(match found {
            Value::List(_) => found,
            other => Value::List(vec![other]),
        });
    }
    match found {
        Value::List(items) => match items.len() {
            0 => Ok(Value::Null),
            1 => Ok(items.into_iter().next().unwrap_or(Value::Null)),
            count => Err(Error::new(format!("find one found {count} rows"))),
        },
        other => Ok(other),
    }
}

fn count_entity(facade: &EntityFacade, name: &str, params: &BTreeMap<String, Value>) -> Result<Value, Error> {
    let names = facade.with_entity(name, |entity, _, _| {
        Ok(entity.fields.iter().map(|field| field.name.clone()).collect::<Vec<_>>())
    })?;
    let mut find = facade.find(name)?;
    for field in names {
        if let Some(value) = params.get(&field) {
            if !is_empty(value) && !matches!(value, Value::Map(_) | Value::List(_)) {
                find = find.eq(&field, value.clone());
            }
        }
    }
    let mut entries = BTreeMap::new();
    entries.insert("count".to_string(), Value::I64(find.count()?));
    Ok(value_map(entries))
}

fn require_user(mode: &str, what: &str) -> Result<(), Error> {
    match mode {
        "false" | "anonymous-all" | "anonymous-view" => Ok(()),
        _ => {
            if crate::user::current_username()?.is_none() {
                Err(Error::of_kind(
                    ErrorKind::AuthenticationRequired,
                    format!("User must be logged in to {what}"),
                ))
            } else {
                Ok(())
            }
        }
    }
}

fn current_facade() -> Result<EntityFacade, Error> {
    ExecutionContext::with_current(|ctx| ctx.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("no entity facade is installed"))
}

fn value_map(entries: BTreeMap<String, Value>) -> Value {
    let stack = moqui_lib::context_stack::ContextStack::new();
    let frame = stack.shared_map();
    {
        let mut map = frame.borrow_mut();
        for (key, value) in entries {
            map.insert(key, value);
        }
    }
    drop(stack);
    unwrap_frame(frame)
}

fn unwrap_frame(frame: moqui_lib::context_stack::Frame) -> Value {
    Value::Map(match Rc::try_unwrap(frame) {
        Ok(cell) => cell.into_inner(),
        Err(frame) => frame.borrow().clone(),
    })
}

/// OpenAPI 3.1 for one service REST root. The caller strips `.json` and rejects YAML.
///
/// # Errors
///
/// Returns an error when the root name is missing or that root is not loaded.
pub(crate) fn openapi_document(segments: &[String]) -> Result<Value, Error> {
    if segments.is_empty() {
        return Err(Error::new("No root resource name specified in path"));
    }
    let roots = current_roots()?;
    let root = roots.resources.get(&segments[0]).ok_or_else(|| {
        Error::new(format!(
            "Service REST API root resource not found with name {}",
            segments[0]
        ))
    })?;
    let mut paths = IndexMap::new();
    collect_paths(root, "", &segments[1..], &mut paths);
    let mut info = IndexMap::new();
    info.insert("title".to_string(), Value::string(&segments[0]));
    info.insert("version".to_string(), Value::string("1.0"));
    let mut server = IndexMap::new();
    server.insert("url".to_string(), Value::string("/rest/s1"));
    let mut doc = IndexMap::new();
    doc.insert("openapi".to_string(), Value::string("3.1.0"));
    doc.insert("info".to_string(), Value::Map(info));
    doc.insert("servers".to_string(), Value::List(vec![Value::Map(server)]));
    doc.insert("paths".to_string(), Value::Map(paths));
    Ok(Value::Map(doc))
}

fn collect_paths(node: &Node, prefix: &str, descend: &[String], paths: &mut IndexMap<String, Value>) {
    let here = join_path(prefix, node);
    if let Some((next, rest)) = descend.split_first() {
        if let Some(child) = node.resources.get(next) {
            collect_paths(child, &here, rest, paths);
            return;
        }
        if let Some(id) = &node.id {
            collect_paths(id, &here, rest, paths);
        }
        return;
    }
    if !node.methods.is_empty() {
        paths.insert(here.clone(), method_object(&node.methods));
    }
    if let Some(id) = &node.id {
        collect_paths(id, &here, &[], paths);
    }
    for child in node.resources.values() {
        collect_paths(child, &here, &[], paths);
    }
}

fn join_path(prefix: &str, node: &Node) -> String {
    let segment = if node.is_id {
        format!("{{{}}}", node.name)
    } else {
        node.name.clone()
    };
    if prefix.is_empty() {
        format!("/{segment}")
    } else {
        format!("{prefix}/{segment}")
    }
}

fn method_object(methods: &BTreeMap<String, Operation>) -> Value {
    let mut map = IndexMap::new();
    for (verb, operation) in methods {
        map.insert(verb.clone(), operation_object(operation));
    }
    Value::Map(map)
}

fn operation_object(operation: &Operation) -> Value {
    let summary = match operation {
        Operation::Service { name, .. } => name.clone(),
        Operation::Entity { name, operation, .. } => format!("{operation} {name}"),
    };
    let mut success = IndexMap::new();
    success.insert("description".to_string(), Value::string("Success"));
    let mut denied = IndexMap::new();
    denied.insert("description".to_string(), Value::string("Authentication required"));
    let mut responses = IndexMap::new();
    responses.insert("200".to_string(), Value::Map(success));
    responses.insert("401".to_string(), Value::Map(denied));
    let mut map = IndexMap::new();
    map.insert("summary".to_string(), Value::string(summary));
    map.insert("responses".to_string(), Value::Map(responses));
    Value::Map(map)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use moqui_lib::object::Value;

    use super::{install, run};
    use crate::context::ExecutionContextFactory;
    use crate::entity::EntityFacade;

    #[test]
    fn basic_enum_types_returns_fixture_rows_and_a_service_runs() {
        let dir = std::env::temp_dir().join(format!("moqui-rest-{}", std::process::id()));
        let _guard = DirGuard(dir.clone());
        std::fs::create_dir_all(dir.join("service")).unwrap();
        std::fs::write(dir.join("service/moqui.rest.xml"), REST_XML).unwrap();
        let service_path = dir.join("service/echo.xml");
        std::fs::write(&service_path, SERVICE_XML).unwrap();
        let facade = crate::testing::open();
        let ctx = ExecutionContextFactory::test_context();
        install(ctx.factory(), &[dir.join("service/moqui.rest.xml")]);
        ctx.factory().set_entity(facade.clone());
        let _entered = ctx.enter();
        let failures = crate::service::lint(std::slice::from_ref(&service_path)).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        insert_type(&facade, "DayOfWeek", "Day of Week");
        insert_type(&facade, "GeoType", "Geo Type");

        let listed = run(&["moqui", "basic", "enumTypes"], "get", &BTreeMap::new()).unwrap();
        assert_eq!(row_count(&listed), 2, "{listed:?}");
        assert_eq!(field(&listed, "DayOfWeek"), Some("Day of Week"));
        assert_eq!(field(&listed, "GeoType"), Some("Geo Type"));

        let one = run(&["moqui", "basic", "enumTypes", "GeoType"], "GET", &BTreeMap::new()).unwrap();
        let Value::Map(one_map) = one else {
            panic!("one result {one:?}");
        };
        assert_eq!(one_map.get("enumTypeId").and_then(Value::as_str), Some("GeoType"));
        assert_eq!(one_map.get("description").and_then(Value::as_str), Some("Geo Type"));

        let mut params = BTreeMap::new();
        params.insert("note".to_string(), Value::string("hello"));
        let echoed = run(&["moqui", "basic", "echo"], "post", &params).unwrap();
        let Value::Map(echoed_map) = echoed else {
            panic!("service result {echoed:?}");
        };
        assert_eq!(echoed_map.get("echoed").and_then(Value::as_str), Some("hello"));

        // The method comes from the request, so a parameter named _method changes nothing.
        let mut sneaky = BTreeMap::new();
        sneaky.insert("_method".to_string(), Value::string("post"));
        let err = run(&["moqui", "basic", "echo"], "get", &sneaky).unwrap_err();
        assert_eq!(
            crate::context::ErrorKind::of(&err),
            crate::context::ErrorKind::MethodNotAllowed,
            "{err}"
        );
        let err = run(&["moqui", "basic", "nothing"], "get", &BTreeMap::new()).unwrap_err();
        assert_eq!(
            crate::context::ErrorKind::of(&err),
            crate::context::ErrorKind::NotFound,
            "{err}"
        );
        let err = run(&["nope"], "get", &BTreeMap::new()).unwrap_err();
        assert_eq!(
            crate::context::ErrorKind::of(&err),
            crate::context::ErrorKind::NotFound,
            "{err}"
        );
    }

    #[test]
    fn a_login_is_required_unless_the_resource_waives_it() {
        let dir = std::env::temp_dir().join(format!("moqui-rest-auth-{}", std::process::id()));
        let _guard = DirGuard(dir.clone());
        std::fs::create_dir_all(dir.join("service")).unwrap();
        std::fs::write(
            dir.join("service/secure.rest.xml"),
            r#"<resource name="secure"><resource name="items">
                <method type="get"><entity name="moqui.basic.EnumerationType" operation="list"/></method>
            </resource></resource>"#,
        )
        .unwrap();
        std::fs::write(dir.join("service/broken.rest.xml"), "<resource name=").unwrap();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(crate::testing::open());
        // A broken file beside it does not stop the good one from being served.
        let mut loaded = BTreeMap::new();
        super::load_dir(&mut loaded, &dir.join("service"));
        assert_eq!(loaded.keys().collect::<Vec<_>>(), ["secure"]);
        install(ctx.factory(), &[dir.join("service/secure.rest.xml")]);
        let _entered = ctx.enter();
        let err = run(&["secure", "items"], "get", &BTreeMap::new()).unwrap_err();
        assert_eq!(
            crate::context::ErrorKind::of(&err),
            crate::context::ErrorKind::AuthenticationRequired,
            "{err}"
        );
    }

    fn insert_type(facade: &EntityFacade, id: &str, description: &str) {
        let mut row = facade.make_value("moqui.basic.EnumerationType").unwrap();
        row.set("enumTypeId", Value::string(id));
        row.set("description", Value::string(description));
        row.create(facade).unwrap();
    }

    fn row_count(value: &Value) -> usize {
        let Value::List(items) = value else {
            panic!("list result {value:?}");
        };
        items.len()
    }

    fn field<'a>(value: &'a Value, id: &str) -> Option<&'a str> {
        let Value::List(items) = value else {
            return None;
        };
        items.iter().find_map(|item| {
            let Value::Map(row) = item else {
                return None;
            };
            let found = row.get("enumTypeId").and_then(Value::as_str)?;
            if found == id {
                row.get("description").and_then(Value::as_str)
            } else {
                None
            }
        })
    }

    struct DirGuard(PathBuf);
    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const REST_XML: &str = r#"<resource name="moqui" require-authentication="false">
        <resource name="basic">
            <resource name="enumTypes">
                <method type="get"><entity name="moqui.basic.EnumerationType" operation="list"/></method>
                <id name="enumTypeId">
                    <method type="get"><entity name="moqui.basic.EnumerationType" operation="one"/></method>
                </id>
            </resource>
            <resource name="echo">
                <method type="post"><service name="echo.ping#Rest"/></method>
            </resource>
        </resource>
    </resource>"#;

    const SERVICE_XML: &str = r#"<services>
        <service verb="ping" noun="Rest" authenticate="false">
            <in-parameters><parameter name="note"/></in-parameters>
            <out-parameters><parameter name="echoed"/></out-parameters>
            <actions><script>echoed = note;</script></actions>
        </service>
    </services>"#;
}
