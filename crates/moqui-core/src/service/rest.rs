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

#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use moqui_lib::node::MNode;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use super::call;
use crate::context::{Error, ExecutionContext};
use crate::entity::EntityFacade;

#[cfg(test)]
thread_local! {
    static EXTRA_FILES: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Walk `path` through `*.rest.xml` and run the `<entity>` or `<service>` operation.
///
/// The first element is the root resource `name`. `_method` selects the verb and
/// defaults to `get`. Component `service/*.rest.xml` files are loaded when an
/// execution context is entered.
pub fn run(path: &[&str], parameters: &BTreeMap<String, Value>) -> Result<Value, Error> {
    if path.is_empty() {
        return Err(Error::new("Cannot run REST service with no path"));
    }
    let method = http_method(parameters);
    let mut params = parameters.clone();
    params.remove("_method");
    let roots = load_roots()?;
    let root = roots.get(path[0]).ok_or_else(|| {
        Error::new(format!(
            "Service REST API root resource not found with name {}",
            path[0]
        ))
    })?;
    root.visit(path, 0, &method, &mut params)
}

fn http_method(parameters: &BTreeMap<String, Value>) -> String {
    parameters
        .get("_method")
        .map(to_plain_string)
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "get".to_string())
        .to_ascii_lowercase()
}

fn load_roots() -> Result<BTreeMap<String, Node>, Error> {
    let mut roots = BTreeMap::new();
    if let Some(dirs) = component_service_dirs() {
        for dir in dirs {
            load_dir(&mut roots, &dir)?;
        }
    }
    #[cfg(test)]
    {
        let extras = EXTRA_FILES.with(|slot| slot.borrow().clone());
        for path in extras {
            insert_file(&mut roots, &path)?;
        }
    }
    Ok(roots)
}

fn component_service_dirs() -> Option<Vec<PathBuf>> {
    ExecutionContext::with_current(|ctx| {
        ctx.factory()
            .components()
            .iter()
            .map(|info| info.location.join("service"))
            .collect()
    })
}

fn load_dir(roots: &mut BTreeMap<String, Node>, dir: &Path) -> Result<(), Error> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|err| Error::new(format!("could not read {}", dir.display())).with_source(err))?
    {
        let entry = entry.map_err(|err| Error::new(format!("could not read {}", dir.display())).with_source(err))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".rest.xml") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    for path in paths {
        insert_file(roots, &path)?;
    }
    Ok(())
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
    roots.insert(node.name.clone(), node);
    Ok(())
}

#[cfg(test)]
fn load_file(path: &Path) -> Result<(), Error> {
    let root = MNode::parse_file(path)?;
    if root.name() != "resource" {
        return Err(Error::new(format!(
            "{} root is {} and not resource",
            path.display(),
            root.name()
        )));
    }
    EXTRA_FILES.with(|slot| slot.borrow_mut().push(path.to_path_buf()));
    Ok(())
}

#[cfg(test)]
fn clear_files() {
    EXTRA_FILES.with(|slot| slot.borrow_mut().clear());
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
            return Err(Error::new(format!(
                "Resource {segment} not valid, index {index} in path {}; resources available are {names}",
                path.join("/")
            )));
        }
        if !self.methods.contains_key(method) {
            if let Some(id) = &self.id {
                if id.allow_extra_path {
                    return id.visit(path, next, method, params);
                }
            }
            return Err(Error::new(format!(
                "Method {method} not supported at {}",
                path.join("/")
            )));
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
                Err(Error::new(format!("User must be logged in to {what}")))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use moqui_lib::object::Value;

    use super::{clear_files, load_file, run};
    use crate::context::ExecutionContextFactory;
    use crate::entity::EntityFacade;

    #[test]
    fn basic_enum_types_returns_fixture_rows_and_a_service_runs() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        clear_files();
        let dir = std::env::temp_dir().join(format!("moqui-rest-{}", std::process::id()));
        let _guard = DirGuard(dir.clone());
        std::fs::create_dir_all(dir.join("service")).unwrap();
        std::fs::write(dir.join("service/moqui.rest.xml"), REST_XML).unwrap();
        let service_path = dir.join("service/echo.xml");
        std::fs::write(&service_path, SERVICE_XML).unwrap();
        let failures = crate::service::define::lint(std::slice::from_ref(&service_path)).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        load_file(&dir.join("service/moqui.rest.xml")).unwrap();

        let facade = mem::open();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(facade.clone());
        let _entered = ctx.enter();
        insert_type(&facade, "DayOfWeek", "Day of Week");
        insert_type(&facade, "GeoType", "Geo Type");

        let listed = run(&["moqui", "basic", "enumTypes"], &BTreeMap::new()).unwrap();
        assert_eq!(row_count(&listed), 2, "{listed:?}");
        assert_eq!(field(&listed, "DayOfWeek"), Some("Day of Week"));
        assert_eq!(field(&listed, "GeoType"), Some("Geo Type"));

        let one = run(&["moqui", "basic", "enumTypes", "GeoType"], &BTreeMap::new()).unwrap();
        let Value::Map(one_map) = one else {
            panic!("one result {one:?}");
        };
        assert_eq!(one_map.get("enumTypeId").and_then(Value::as_str), Some("GeoType"));
        assert_eq!(one_map.get("description").and_then(Value::as_str), Some("Geo Type"));

        let mut params = BTreeMap::new();
        params.insert("_method".to_string(), Value::string("post"));
        params.insert("note".to_string(), Value::string("hello"));
        let echoed = run(&["moqui", "basic", "echo"], &params).unwrap();
        let Value::Map(echoed_map) = echoed else {
            panic!("service result {echoed:?}");
        };
        assert_eq!(echoed_map.get("echoed").and_then(Value::as_str), Some("hello"));
        clear_files();
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
            clear_files();
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

    mod mem {
        use std::collections::{BTreeMap, BTreeSet};
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use moqui_lib::error::Error;
        use moqui_lib::object::Value;

        use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
        use crate::entity::{EntityFacade, EntityGroup};
        use crate::transaction::{PoolConfig, TransactionManager};

        type Row = BTreeMap<String, Value>;
        type Tables = BTreeMap<String, Vec<Row>>;

        pub(super) fn open() -> EntityFacade {
            let driver: Arc<dyn DbDriver> = Arc::new(MemDriver {
                store: Arc::new(Mutex::new(BTreeMap::new())),
            });
            let transactions = TransactionManager::new(
                driver,
                PoolConfig {
                    min: 0,
                    max: 4,
                    reserve: 2,
                    acquire_timeout: Duration::from_secs(5),
                },
            )
            .expect("memory pool");
            let facade = EntityFacade::open(
                &framework_dir(),
                "transactional",
                vec![EntityGroup::new("transactional", "postgres", transactions)],
            )
            .expect("entity facade");
            facade.set_disable_audit_log(true);
            facade.set_disable_entity_eca(true);
            facade.set_disable_fk_create(true);
            facade.set_disable_data_feed(true);
            facade
        }

        fn framework_dir() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
        }

        struct MemDriver {
            store: Arc<Mutex<Tables>>,
        }

        struct MemConn {
            store: Arc<Mutex<Tables>>,
            local: Option<Tables>,
            dirty: BTreeSet<String>,
        }

        struct NoCancel;

        impl DbCancel for NoCancel {
            fn cancel(&self) -> Result<(), Error> {
                Ok(())
            }
        }

        impl DbDriver for MemDriver {
            fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
                Ok(Box::new(MemConn {
                    store: Arc::clone(&self.store),
                    local: None,
                    dirty: BTreeSet::new(),
                }))
            }
        }

        impl DbConnection for MemConn {
            fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
                Ok(self.apply(sql, params)?.0)
            }

            fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
                Ok(self.apply(sql, params)?.1)
            }

            fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
                self.query(sql, params)
            }

            fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
                Ok(())
            }

            fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
                Ok(None)
            }

            fn close_stream(&mut self) -> Result<(), Error> {
                Ok(())
            }

            fn stream_open(&self) -> bool {
                false
            }

            fn begin(&mut self) -> Result<(), Error> {
                if self.local.is_none() {
                    let committed = self.store.lock().unwrap_or_else(|err| err.into_inner()).clone();
                    self.local = Some(committed);
                    self.dirty.clear();
                }
                Ok(())
            }

            fn commit(&mut self) -> Result<(), Error> {
                let Some(local) = self.local.take() else {
                    return Ok(());
                };
                let dirty = std::mem::take(&mut self.dirty);
                let mut committed = self.store.lock().unwrap_or_else(|err| err.into_inner());
                for table in dirty {
                    let rows = local.get(&table).cloned().unwrap_or_default();
                    committed.insert(table, rows);
                }
                Ok(())
            }

            fn rollback(&mut self) -> Result<(), Error> {
                self.local = None;
                self.dirty.clear();
                Ok(())
            }

            fn savepoint(&mut self, _name: &str) -> Result<(), Error> {
                Ok(())
            }

            fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), Error> {
                Ok(())
            }

            fn release_savepoint(&mut self, _name: &str) -> Result<(), Error> {
                Ok(())
            }

            fn canceller(&self) -> Box<dyn DbCancel> {
                Box::new(NoCancel)
            }
        }

        impl MemConn {
            fn apply(&mut self, sql: &str, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                let sql = sql.trim();
                if sql.eq_ignore_ascii_case("SELECT 1") {
                    return Ok((
                        1,
                        vec![DbRow {
                            columns: vec![("1".to_string(), Value::I64(1))],
                        }],
                    ));
                }
                let stmt = parse(sql).map_err(|err| Error::new(format!("{err} sql={sql}")))?;
                self.exec(stmt, params)
            }

            fn exec(&mut self, stmt: Stmt, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                match stmt {
                    Stmt::Insert { table, columns } => {
                        let mut row = BTreeMap::new();
                        for (index, column) in columns.iter().enumerate() {
                            let value = params
                                .get(index)
                                .cloned()
                                .ok_or_else(|| Error::new("insert is missing a parameter"))?;
                            row.insert(column.clone(), value);
                        }
                        let mut rows = self.read_table(&table);
                        rows.push(row);
                        self.put_table(&table, rows);
                        Ok((1, Vec::new()))
                    }
                    Stmt::Select(query) => self.exec_select(&query, params),
                }
            }

            fn exec_select(&mut self, query: &SelectQuery, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                let mut matched = Vec::new();
                for row in self.read_table(&query.table) {
                    if keep(&query.pred, &row, params)? {
                        matched.push(row);
                    }
                }
                if query.count {
                    let count = i64::try_from(matched.len()).unwrap_or(i64::MAX);
                    return Ok((
                        1,
                        vec![DbRow {
                            columns: vec![("count".to_string(), Value::I64(count))],
                        }],
                    ));
                }
                let mut idx = pred_slots(&query.pred);
                let take_n = if query.limit {
                    Some(as_i64(take_param(params, &mut idx)?)?)
                } else {
                    None
                };
                let skip = if query.offset {
                    as_i64(take_param(params, &mut idx)?)?
                } else {
                    0
                };
                let start = usize::try_from(skip.max(0)).unwrap_or(0);
                let window: Vec<Row> = match take_n {
                    Some(limit) => {
                        let len = usize::try_from(limit.max(0)).unwrap_or(0);
                        matched.into_iter().skip(start).take(len).collect()
                    }
                    None => matched.into_iter().skip(start).collect(),
                };
                let projected = window
                    .iter()
                    .map(|row| DbRow {
                        columns: query
                            .columns
                            .iter()
                            .map(|name| (name.clone(), row.get(name).cloned().unwrap_or(Value::Null)))
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                let count = u64::try_from(projected.len()).unwrap_or(u64::MAX);
                Ok((count, projected))
            }

            fn read_table(&self, table: &str) -> Vec<Row> {
                if let Some(local) = &self.local {
                    return local.get(table).cloned().unwrap_or_default();
                }
                self.store
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .get(table)
                    .cloned()
                    .unwrap_or_default()
            }

            fn put_table(&mut self, table: &str, rows: Vec<Row>) {
                if let Some(local) = self.local.as_mut() {
                    local.insert(table.to_string(), rows);
                    self.dirty.insert(table.to_string());
                    return;
                }
                self.store
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .insert(table.to_string(), rows);
            }
        }

        fn keep(pred: &Option<Pred>, row: &Row, params: &[Value]) -> Result<bool, Error> {
            let Some(pred) = pred else {
                return Ok(true);
            };
            let mut idx = 0;
            eval_pred(pred, row, params, &mut idx)
        }

        fn pred_slots(pred: &Option<Pred>) -> usize {
            pred.as_ref().map(count_slots).unwrap_or(0)
        }

        fn count_slots(pred: &Pred) -> usize {
            match pred {
                Pred::Cmp { .. } => 1,
                Pred::And(parts) | Pred::Or(parts) => parts.iter().map(count_slots).sum(),
            }
        }

        fn eval_pred(pred: &Pred, row: &Row, params: &[Value], idx: &mut usize) -> Result<bool, Error> {
            match pred {
                Pred::Cmp { field } => {
                    let param = take_param(params, idx)?;
                    Ok(row.get(field) == Some(param))
                }
                Pred::And(parts) => {
                    let mut all = true;
                    for part in parts {
                        if !eval_pred(part, row, params, idx)? {
                            all = false;
                        }
                    }
                    Ok(all)
                }
                Pred::Or(parts) => {
                    let mut any = false;
                    for part in parts {
                        if eval_pred(part, row, params, idx)? {
                            any = true;
                        }
                    }
                    Ok(any)
                }
            }
        }

        fn take_param<'a>(params: &'a [Value], idx: &mut usize) -> Result<&'a Value, Error> {
            let value = params.get(*idx).ok_or_else(|| Error::new("sql parameter missing"))?;
            *idx += 1;
            Ok(value)
        }

        fn as_i64(value: &Value) -> Result<i64, Error> {
            match value {
                Value::I64(number) => Ok(*number),
                Value::I32(number) => Ok(i64::from(*number)),
                Value::String(text) => text.parse().map_err(|_| Error::new("parameter is not an integer")),
                _ => Err(Error::new("parameter is not an integer")),
            }
        }

        enum Stmt {
            Insert { table: String, columns: Vec<String> },
            Select(SelectQuery),
        }

        struct SelectQuery {
            count: bool,
            columns: Vec<String>,
            table: String,
            pred: Option<Pred>,
            limit: bool,
            offset: bool,
        }

        enum Pred {
            Cmp { field: String },
            And(Vec<Pred>),
            Or(Vec<Pred>),
        }

        fn parse(sql: &str) -> Result<Stmt, Error> {
            let mut cur = Cur { sql, i: 0 };
            if cur.eat_keyword("INSERT") {
                return parse_insert(&mut cur);
            }
            if cur.eat_keyword("SELECT") {
                return parse_select(&mut cur);
            }
            Err(cur.fail("unsupported sql"))
        }

        fn parse_insert(cur: &mut Cur) -> Result<Stmt, Error> {
            cur.expect_keyword("INTO")?;
            let table = cur.quoted()?;
            cur.expect_symbol(b'(')?;
            let mut columns = Vec::new();
            loop {
                columns.push(cur.quoted()?);
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            cur.expect_symbol(b')')?;
            cur.expect_keyword("VALUES")?;
            cur.expect_symbol(b'(')?;
            for index in 0..columns.len() {
                if index > 0 {
                    cur.expect_symbol(b',')?;
                }
                cur.expect_symbol(b'?')?;
            }
            cur.expect_symbol(b')')?;
            cur.expect_end()?;
            Ok(Stmt::Insert { table, columns })
        }

        fn parse_select(cur: &mut Cur) -> Result<Stmt, Error> {
            let count = if cur.eat_keyword("COUNT") {
                cur.expect_symbol(b'(')?;
                cur.expect_symbol(b'*')?;
                cur.expect_symbol(b')')?;
                true
            } else {
                false
            };
            let columns = if count { Vec::new() } else { parse_columns(cur)? };
            cur.expect_keyword("FROM")?;
            let table = cur.quoted()?;
            let pred = if cur.eat_keyword("WHERE") {
                Some(parse_or(cur)?)
            } else {
                None
            };
            if cur.eat_keyword("ORDER") {
                return Err(cur.fail("order by is not used by this fixture"));
            }
            let limit = if cur.eat_keyword("LIMIT") {
                cur.expect_symbol(b'?')?;
                true
            } else {
                false
            };
            let offset = if cur.eat_keyword("OFFSET") {
                cur.expect_symbol(b'?')?;
                true
            } else {
                false
            };
            cur.expect_end()?;
            Ok(Stmt::Select(SelectQuery {
                count,
                columns,
                table,
                pred,
                limit,
                offset,
            }))
        }

        fn parse_columns(cur: &mut Cur) -> Result<Vec<String>, Error> {
            let mut columns = Vec::new();
            loop {
                if cur.peek_keyword("FROM") {
                    break;
                }
                columns.push(cur.quoted()?);
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            if columns.is_empty() {
                return Err(cur.fail("select has no columns"));
            }
            Ok(columns)
        }

        fn parse_or(cur: &mut Cur) -> Result<Pred, Error> {
            let mut parts = vec![parse_and(cur)?];
            while cur.eat_keyword("OR") {
                parts.push(parse_and(cur)?);
            }
            Ok(fold(parts, false))
        }

        fn parse_and(cur: &mut Cur) -> Result<Pred, Error> {
            let mut parts = vec![parse_primary(cur)?];
            while cur.eat_keyword("AND") {
                parts.push(parse_primary(cur)?);
            }
            Ok(fold(parts, true))
        }

        fn parse_primary(cur: &mut Cur) -> Result<Pred, Error> {
            if cur.eat_symbol(b'(') {
                let pred = parse_or(cur)?;
                cur.expect_symbol(b')')?;
                return Ok(pred);
            }
            let field = cur.quoted()?;
            cur.expect_symbol(b'=')?;
            cur.expect_symbol(b'?')?;
            Ok(Pred::Cmp { field })
        }

        fn fold(mut parts: Vec<Pred>, and: bool) -> Pred {
            if parts.len() == 1 {
                return parts.remove(0);
            }
            if and {
                Pred::And(parts)
            } else {
                Pred::Or(parts)
            }
        }

        struct Cur<'a> {
            sql: &'a str,
            i: usize,
        }

        impl Cur<'_> {
            fn skip(&mut self) {
                while self.i < self.sql.len() && self.sql.as_bytes()[self.i].is_ascii_whitespace() {
                    self.i += 1;
                }
            }

            fn eat_keyword(&mut self, word: &str) -> bool {
                self.skip();
                let rest = &self.sql[self.i..];
                if rest.len() < word.len() || !rest.starts_with(word) {
                    return false;
                }
                let end = self.i + word.len();
                if end < self.sql.len() {
                    let next = self.sql.as_bytes()[end];
                    if next.is_ascii_alphanumeric() || next == b'_' {
                        return false;
                    }
                }
                self.i = end;
                true
            }

            fn peek_keyword(&mut self, word: &str) -> bool {
                let saved = self.i;
                let found = self.eat_keyword(word);
                self.i = saved;
                found
            }

            fn eat_symbol(&mut self, symbol: u8) -> bool {
                self.skip();
                if self.i < self.sql.len() && self.sql.as_bytes()[self.i] == symbol {
                    self.i += 1;
                    true
                } else {
                    false
                }
            }

            fn quoted(&mut self) -> Result<String, Error> {
                if !self.eat_symbol(b'"') {
                    return Err(self.fail("expected identifier"));
                }
                let start = self.i;
                while self.i < self.sql.len() && self.sql.as_bytes()[self.i] != b'"' {
                    self.i += 1;
                }
                if self.i >= self.sql.len() {
                    return Err(self.fail("unterminated identifier"));
                }
                let name = self.sql[start..self.i].to_string();
                self.i += 1;
                Ok(name)
            }

            fn expect_keyword(&mut self, word: &str) -> Result<(), Error> {
                if self.eat_keyword(word) {
                    Ok(())
                } else {
                    Err(self.fail(&format!("expected {word}")))
                }
            }

            fn expect_symbol(&mut self, symbol: u8) -> Result<(), Error> {
                if self.eat_symbol(symbol) {
                    Ok(())
                } else {
                    Err(self.fail(&format!("expected {}", char::from(symbol))))
                }
            }

            fn expect_end(&mut self) -> Result<(), Error> {
                self.skip();
                if self.i == self.sql.len() {
                    Ok(())
                } else {
                    Err(self.fail("unexpected sql"))
                }
            }

            fn fail(&self, message: &str) -> Error {
                let rest: String = self.sql[self.i..].chars().take(160).collect();
                Error::new(format!("{message} at '{rest}'"))
            }
        }
    }
}
