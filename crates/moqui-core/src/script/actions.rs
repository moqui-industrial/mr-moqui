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

//! XML actions. One cached script per action XML text. Names in that script are bare.

use std::cell::RefCell;
use std::collections::HashMap;

use moqui_lib::node::MNode;
use rquickjs::{Array, CatchResultExt, Ctx, Function, Object};

use crate::context::Error;

const XML_ACTIONS: &str = include_str!("../../../../framework/template/XmlActions.js");

const MAX_CACHED_SCRIPTS: usize = 4096;

thread_local! {
    static SCRIPTS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

pub fn run_actions(actions: &str) -> Result<(), Error> {
    if actions.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    let source = generated(actions)?;
    super::runtime::eval_script(&source)
}

fn generated(xml: &str) -> Result<String, Error> {
    let cached = SCRIPTS.with(|scripts| scripts.borrow().get(xml).cloned());
    if let Some(source) = cached {
        return Ok(source);
    }
    let source = generate(xml)?;
    SCRIPTS.with(|scripts| {
        let mut scripts = scripts.borrow_mut();
        // Dynamic action text would otherwise grow this without bound.
        if scripts.len() >= MAX_CACHED_SCRIPTS {
            scripts.clear();
        }
        scripts.insert(xml.to_string(), source.clone());
    });
    Ok(source)
}

fn generate(xml: &str) -> Result<String, Error> {
    let root = MNode::parse_text("actions", xml)?;
    reject(&root)?;
    super::runtime::with_ctx(|ctx| {
        let macros: Object = super::runtime::eval_source(ctx, XML_ACTIONS)?;
        let (source, _) = render(ctx, &macros, &root)?;
        Ok(source)
    })
}

fn reject(node: &MNode) -> Result<(), Error> {
    let name = node.name();
    if name == "entity-data" {
        return Err(Error::new("entity-data is not implemented"));
    }
    if name == "service-call" && node.attribute("async").as_deref() == Some("distribute") {
        return Err(Error::new("async=\"distribute\" is not a host method"));
    }
    for child in node.children() {
        reject(&child)?;
    }
    Ok(())
}

fn render<'js>(ctx: &Ctx<'js>, macros: &Object<'js>, node: &MNode) -> Result<(String, Object<'js>), Error> {
    let children = Array::new(ctx.clone()).map_err(js_err)?;
    let mut combined = String::new();
    for (index, child) in node.children().iter().enumerate() {
        let (source, child_obj) = render(ctx, macros, child)?;
        combined.push_str(&source);
        children.set(index, child_obj).map_err(js_err)?;
    }
    let node_obj = Object::new(ctx.clone()).map_err(js_err)?;
    node_obj.set("name", node.name()).map_err(js_err)?;
    node_obj.set("text", node.text().unwrap_or_default()).map_err(js_err)?;
    node_obj.set("attrs", attrs_object(ctx, node)?).map_err(js_err)?;
    node_obj.set("children", children).map_err(js_err)?;
    let name = node.name();
    if !macros.contains_key(name.as_str()).map_err(js_err)? {
        return Err(Error::new(format!("no XML action macro for {name}")));
    }
    let function: Function = macros.get(name.as_str()).map_err(js_err)?;
    let source: String = function
        .call((node_obj.clone(), combined))
        .catch(ctx)
        .map_err(|caught| Error::new(caught.to_string()))?;
    node_obj.set("source", source.clone()).map_err(js_err)?;
    Ok((source, node_obj))
}

fn attrs_object<'js>(ctx: &Ctx<'js>, node: &MNode) -> Result<Object<'js>, Error> {
    let attrs = Object::new(ctx.clone()).map_err(js_err)?;
    for (key, value) in node.attributes() {
        attrs.set(key, value).map_err(js_err)?;
    }
    Ok(attrs)
}

fn js_err(err: impl ToString) -> Error {
    Error::new(err.to_string())
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use moqui_lib::error::Error as LibError;
    use moqui_lib::object::{compare_obj, Value};
    use rquickjs::Object;

    use super::{generate, run_actions, XML_ACTIONS};
    use crate::context::{Error, ExecutionContext, ExecutionContextFactory};
    use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use crate::entity::{EntityFacade, EntityGroup, EntityValue};
    use crate::script::runtime::{clear_context_stack, eval_condition, set_context_name};
    use crate::transaction::{PoolConfig, TransactionManager};

    const MACROS: &[&str] = &[
        "actions",
        "and",
        "assert",
        "break",
        "check-errors",
        "compare",
        "condition",
        "continue",
        "date-filter",
        "default-parameters",
        "econdition",
        "econdition-object",
        "econditions",
        "else",
        "else-if",
        "entity-create",
        "entity-data",
        "entity-delete",
        "entity-delete-by-condition",
        "entity-delete-related",
        "entity-find",
        "entity-find-count",
        "entity-find-one",
        "entity-find-related",
        "entity-find-related-one",
        "entity-make-value",
        "entity-sequenced-id-primary",
        "entity-sequenced-id-secondary",
        "entity-set",
        "entity-update",
        "expression",
        "field-map",
        "filter-map-list",
        "having-econditions",
        "if",
        "iterate",
        "limit-range",
        "limit-view",
        "log",
        "message",
        "not",
        "or",
        "order-by",
        "order-map-list",
        "return",
        "script",
        "search-form-inputs",
        "select-field",
        "service-call",
        "set",
        "then",
        "use-iterator",
        "while",
    ];

    const INLINE: &str = r#"<actions>
    <entity-find entity-name="moqui.test.TestEntity" list="found">
        <econdition field-name="testId" from="seedId"/>
    </entity-find>
    <entity-find-one entity-name="moqui.test.TestEntity" value-field="oneRow">
        <field-map field-name="testId" from="seedId"/>
    </entity-find-one>
    <script>foundCount = found.length; foundId = found.get(0).get("testId"); oneId = oneRow.get("testId");</script>
    <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
    <script>made.set("testId", "B"); made.set("testMedium", "made");</script>
    <entity-create value-field="made"/>
</actions>"#;

    fn assert_golden(xml: &str, expected: &str) {
        assert_eq!(generate(xml).unwrap(), expected);
        assert_eq!(generate(xml).unwrap(), expected);
    }

    #[test]
    fn list_helpers_and_the_condition_factory_run() {
        clear_context_stack();
        run_actions(
            r#"<actions>
                <set field="items" from="[{n: 'b', q: 2}, {n: 'c', q: 2}, {n: 'a', q: 1}]"/>
                <order-map-list list="items"><order-by field-name="n"/></order-map-list>
                <filter-map-list list="items"><field-map field-name="q" value="2"/></filter-map-list>
                <set field="names" from="items.map(function (item) { return item.n; }).join(',')"/>
                <set field="seesNames" from="context.names"/>
                <set field="made" from="ec.entity.conditionFactory.makeCondition('a', 'greater', 3) != null"/>
                <set field="skipped" from="ec.entity.conditionFactory.makeActionConditionDirect('a', 'equals', null, null, null, false, true, false, 'false') === null"/>
                <set field="dated" from="ec.entity.conditionFactory.makeConditionDate('fromDate', 'thruDate', null, false, 'false') != null"/>
            </actions>"#,
        )
        .unwrap();
        let text =
            |name: &str| crate::script::context_value(name).map(|value| moqui_lib::object::to_plain_string(&value));
        assert_eq!(text("names").as_deref(), Some("b,c"));
        assert_eq!(text("seesNames").as_deref(), Some("b,c"));
        assert_eq!(text("made").as_deref(), Some("true"));
        assert_eq!(text("skipped").as_deref(), Some("true"));
        assert_eq!(text("dated").as_deref(), Some("true"));
    }

    #[test]
    fn golden_service_call() {
        assert_golden(
            include_str!("testdata/service-call.xml"),
            include_str!("testdata/service-call.js"),
        );
    }

    #[test]
    fn golden_script() {
        assert_golden(include_str!("testdata/script.xml"), include_str!("testdata/script.js"));
    }

    #[test]
    fn golden_set() {
        assert_golden(include_str!("testdata/set.xml"), include_str!("testdata/set.js"));
    }

    #[test]
    fn golden_entity_find() {
        assert_golden(
            include_str!("testdata/entity-find.xml"),
            include_str!("testdata/entity-find.js"),
        );
    }

    #[test]
    fn golden_entity_value() {
        assert_golden(
            include_str!("testdata/entity-value.xml"),
            include_str!("testdata/entity-value.js"),
        );
    }

    #[test]
    fn golden_control() {
        assert_golden(
            include_str!("testdata/control.xml"),
            include_str!("testdata/control.js"),
        );
    }

    #[test]
    fn golden_message() {
        assert_golden(
            include_str!("testdata/message.xml"),
            include_str!("testdata/message.js"),
        );
    }

    #[test]
    fn golden_condition() {
        assert_golden(
            include_str!("testdata/condition.xml"),
            include_str!("testdata/condition.js"),
        );
    }

    #[test]
    fn each_concrete_element_has_a_macro() {
        let names = macro_names().unwrap();
        assert_eq!(names, MACROS);
        assert!(!names
            .iter()
            .any(|name| *name == "service-group" || *name == "service-invoke"));
    }

    #[test]
    fn empty_actions_are_ok() {
        run_actions("<actions/>").unwrap();
    }

    #[test]
    fn distribute_is_a_load_error() {
        let err =
            run_actions(r#"<actions><service-call name="example.do#It" async="distribute"/></actions>"#).unwrap_err();
        assert!(err.to_string().contains("distribute"), "{err}");
    }

    #[test]
    fn entity_data_is_not_implemented() {
        let err = run_actions("<actions><entity-data/></actions>").unwrap_err();
        assert!(err.to_string().contains("entity-data"), "{err}");
    }

    #[test]
    fn service_async_has_no_distribute_method() {
        clear_context_stack();
        let present =
            eval_condition("typeof ec.service.async === \"function\" && ec.service.async().distribute == null")
                .unwrap();
        assert!(present);
    }

    #[test]
    fn entity_find_without_an_execution_context_errors() {
        clear_context_stack();
        let err = run_actions(
            r#"<actions><entity-find entity-name="moqui.test.TestEntity" list="found"><econdition field-name="testId" from="seedId"/></entity-find></actions>"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no execution context is entered"), "{err}");
    }

    #[test]
    fn message_expands_and_records_an_error() {
        clear_context_stack();
        let ctx = ExecutionContextFactory::test_context();
        let _guard = ctx.enter();
        set_context_name("seedId", Value::string("A")).unwrap();
        run_actions(r#"<actions><message error="true">failed ${seedId}</message></actions>"#).unwrap();
        let errors = ExecutionContext::with_current(|ec| ec.message().errors().to_vec()).unwrap();
        assert_eq!(errors, vec!["failed A".to_string()]);
        clear_context_stack();
    }

    #[test]
    fn inline_action_finds_and_creates() -> Result<(), Error> {
        clear_context_stack();
        let facade = open_facade();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(facade.clone());
        let _guard = ctx.enter();
        let mut seed = facade.make_value("moqui.test.TestEntity").expect("make seed");
        seed.set("testId", Value::string("A"));
        seed.set("testMedium", Value::string("seed"));
        seed.create(&facade).expect("create seed");
        set_context_name("seedId", Value::string("A"))?;
        run_actions(INLINE)?;
        assert!(eval_condition("foundCount === 1")?, "foundCount");
        assert!(eval_condition("foundId === \"A\"")?, "foundId");
        assert!(eval_condition("oneId === \"A\"")?, "oneId");
        let rows = facade
            .find("moqui.test.TestEntity")
            .expect("find")
            .list()
            .expect("list");
        let mut ids = Vec::new();
        for row in rows.iter() {
            ids.push(field_str(row, "testId"));
        }
        ids.sort();
        assert_eq!(ids, vec!["A".to_string(), "B".to_string()]);
        let made = rows.iter().find(|row| field_str(row, "testId") == "B").expect("row B");
        assert_eq!(field_str(made, "testMedium"), "made");
        clear_context_stack();
        Ok(())
    }

    fn macro_names() -> Result<Vec<String>, Error> {
        crate::script::runtime::with_ctx(|ctx| {
            let macros: Object = crate::script::runtime::eval_source(ctx, XML_ACTIONS)?;
            let mut names = Vec::new();
            for key in macros.keys::<String>() {
                names.push(key.map_err(|err| Error::new(err.to_string()))?);
            }
            names.sort();
            Ok(names)
        })
    }

    fn field_str(row: &EntityValue, name: &str) -> String {
        row.get(name).and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn open_facade() -> EntityFacade {
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

    type Row = BTreeMap<String, Value>;
    type Tables = BTreeMap<String, Vec<Row>>;

    struct MemDriver {
        store: Arc<Mutex<Tables>>,
    }

    struct MemConn {
        store: Arc<Mutex<Tables>>,
    }

    struct NoCancel;

    impl DbCancel for NoCancel {
        fn cancel(&self) -> Result<(), LibError> {
            Ok(())
        }
    }

    impl DbDriver for MemDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, LibError> {
            Ok(Box::new(MemConn {
                store: Arc::clone(&self.store),
            }))
        }
    }

    impl DbConnection for MemConn {
        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, LibError> {
            Ok(self.apply(sql, params)?.0)
        }

        fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, LibError> {
            Ok(self.apply(sql, params)?.1)
        }

        fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, LibError> {
            self.query(sql, params)
        }

        fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), LibError> {
            Ok(())
        }

        fn next_row(&mut self) -> Result<Option<DbRow>, LibError> {
            Ok(None)
        }

        fn close_stream(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn stream_open(&self) -> bool {
            false
        }

        fn begin(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn commit(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn release_savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            Box::new(NoCancel)
        }
    }

    impl MemConn {
        fn apply(&mut self, sql: &str, params: &[Value]) -> Result<(u64, Vec<DbRow>), LibError> {
            let sql = sql.trim();
            if sql.eq_ignore_ascii_case("SELECT 1") {
                return Ok((
                    1,
                    vec![DbRow {
                        columns: vec![("1".to_string(), Value::I64(1))],
                    }],
                ));
            }
            let stmt = parse_sql(sql)?;
            match stmt {
                Stmt::Insert { table, columns } => {
                    let mut row = BTreeMap::new();
                    for (index, column) in columns.iter().enumerate() {
                        let value = params
                            .get(index)
                            .cloned()
                            .ok_or_else(|| LibError::new("insert is missing a parameter"))?;
                        row.insert(column.clone(), value);
                    }
                    let mut tables = self.store.lock().unwrap_or_else(|err| err.into_inner());
                    tables.entry(table).or_default().push(row);
                    Ok((1, Vec::new()))
                }
                Stmt::Select(query) => select_rows(&self.store, &query, params),
            }
        }
    }

    fn select_rows(
        store: &Mutex<Tables>,
        query: &SelectQuery,
        params: &[Value],
    ) -> Result<(u64, Vec<DbRow>), LibError> {
        let tables = store.lock().unwrap_or_else(|err| err.into_inner());
        let rows = tables.get(&query.table).cloned().unwrap_or_default();
        drop(tables);
        let mut matched = Vec::new();
        for row in rows {
            if keep(&query.pred, &row, params, 0)? {
                matched.push(row);
            }
        }
        if query.count {
            let n = i64::try_from(matched.len()).unwrap_or(i64::MAX);
            return Ok((
                1,
                vec![DbRow {
                    columns: vec![("count".to_string(), Value::I64(n))],
                }],
            ));
        }
        matched.sort_by(|left, right| sort_rows(left, right, &query.order));
        // Postgres writes `LIMIT ? OFFSET ?`, so the limit parameter is first.
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
        let projected: Vec<DbRow> = window.iter().map(|row| project(row, &query.columns)).collect();
        let n = u64::try_from(projected.len()).unwrap_or(u64::MAX);
        Ok((n, projected))
    }

    fn project(row: &Row, columns: &[String]) -> DbRow {
        DbRow {
            columns: columns
                .iter()
                .map(|name| {
                    let value = row.get(name).cloned().unwrap_or(Value::Null);
                    (name.clone(), value)
                })
                .collect(),
        }
    }

    fn keep(pred: &Option<Pred>, row: &Row, params: &[Value], start: usize) -> Result<bool, LibError> {
        let Some(pred) = pred else {
            return Ok(true);
        };
        let mut idx = start;
        eval_pred(pred, row, params, &mut idx)
    }

    fn pred_slots(pred: &Option<Pred>) -> usize {
        pred.as_ref().map(count_slots).unwrap_or(0)
    }

    fn count_slots(pred: &Pred) -> usize {
        match pred {
            Pred::Cmp { .. } => 1,
            Pred::IsNull { .. } => 0,
            Pred::And(parts) | Pred::Or(parts) => parts.iter().map(count_slots).sum(),
        }
    }

    fn eval_pred(pred: &Pred, row: &Row, params: &[Value], idx: &mut usize) -> Result<bool, LibError> {
        match pred {
            Pred::Cmp { field, op } => {
                let param = take_param(params, idx)?;
                let left = row.get(field).cloned().unwrap_or(Value::Null);
                Ok(compare_values(&left, *op, param))
            }
            Pred::IsNull { field, negated } => {
                let is_null = row.get(field).is_none_or(Value::is_null);
                Ok(if *negated { !is_null } else { is_null })
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

    fn compare_values(left: &Value, op: Op, right: &Value) -> bool {
        let ordering = compare_obj(left, right).unwrap_or(Ordering::Equal);
        match op {
            Op::Eq => left == right,
            Op::Ne => left != right,
            Op::Lt => ordering == Ordering::Less,
            Op::Gt => ordering == Ordering::Greater,
            Op::Le => ordering != Ordering::Greater,
            Op::Ge => ordering != Ordering::Less,
        }
    }

    fn sort_rows(left: &Row, right: &Row, order: &[(String, bool)]) -> Ordering {
        for (field, descending) in order {
            let left = left.get(field).cloned().unwrap_or(Value::Null);
            let right = right.get(field).cloned().unwrap_or(Value::Null);
            let mut ordering = compare_obj(&left, &right).unwrap_or(Ordering::Equal);
            if *descending {
                ordering = ordering.reverse();
            }
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    }

    fn take_param<'a>(params: &'a [Value], idx: &mut usize) -> Result<&'a Value, LibError> {
        let value = params
            .get(*idx)
            .ok_or_else(|| LibError::new("sql is missing a parameter"))?;
        *idx += 1;
        Ok(value)
    }

    fn as_i64(value: &Value) -> Result<i64, LibError> {
        match value {
            Value::I64(number) => Ok(*number),
            Value::I32(number) => Ok(i64::from(*number)),
            other => Err(LibError::new(format!("expected an integer parameter, found {other:?}"))),
        }
    }

    enum Stmt {
        Insert { table: String, columns: Vec<String> },
        Select(SelectQuery),
    }

    struct SelectQuery {
        columns: Vec<String>,
        count: bool,
        table: String,
        pred: Option<Pred>,
        order: Vec<(String, bool)>,
        limit: bool,
        offset: bool,
    }

    enum Pred {
        Cmp { field: String, op: Op },
        IsNull { field: String, negated: bool },
        And(Vec<Pred>),
        Or(Vec<Pred>),
    }

    #[derive(Clone, Copy)]
    enum Op {
        Eq,
        Ne,
        Lt,
        Gt,
        Le,
        Ge,
    }

    fn parse_sql(sql: &str) -> Result<Stmt, LibError> {
        parse_stmt(sql).map_err(|err| LibError::new(format!("{err} sql={sql}")))
    }

    struct Cur<'a> {
        raw: &'a str,
        at: usize,
    }

    impl<'a> Cur<'a> {
        fn new(raw: &'a str) -> Self {
            Self { raw, at: 0 }
        }

        fn peek(&self) -> Option<char> {
            self.raw[self.at..].chars().next()
        }

        fn bump(&mut self) -> Option<char> {
            let ch = self.peek()?;
            self.at += ch.len_utf8();
            Some(ch)
        }

        fn skip_ws(&mut self) {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
        }

        fn eat_word(&mut self, word: &str) -> bool {
            self.skip_ws();
            let rest = &self.raw[self.at..];
            if rest.len() < word.len() || !rest[..word.len()].eq_ignore_ascii_case(word) {
                return false;
            }
            let after = self.at + word.len();
            if self.raw[after..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                return false;
            }
            self.at = after;
            true
        }

        fn quoted(&mut self) -> Result<String, String> {
            self.skip_ws();
            if self.bump() != Some('"') {
                return Err("expected a quoted identifier".to_string());
            }
            let mut out = String::new();
            loop {
                match self.bump() {
                    Some('"') => {
                        if self.peek() == Some('"') {
                            self.bump();
                            out.push('"');
                        } else {
                            return Ok(out);
                        }
                    }
                    Some(ch) => out.push(ch),
                    None => return Err("unterminated identifier".to_string()),
                }
            }
        }
    }

    fn parse_stmt(sql: &str) -> Result<Stmt, String> {
        let mut cur = Cur::new(sql);
        if cur.eat_word("INSERT") {
            return parse_insert(&mut cur);
        }
        if cur.eat_word("SELECT") {
            return parse_select(&mut cur).map(Stmt::Select);
        }
        Err("expected INSERT or SELECT".to_string())
    }

    fn parse_insert(cur: &mut Cur<'_>) -> Result<Stmt, String> {
        if !cur.eat_word("INTO") {
            return Err("expected INTO".to_string());
        }
        let table = cur.quoted()?;
        cur.skip_ws();
        if cur.bump() != Some('(') {
            return Err("expected (".to_string());
        }
        let mut columns = Vec::new();
        loop {
            columns.push(cur.quoted()?);
            cur.skip_ws();
            if cur.peek() == Some(',') {
                cur.bump();
                continue;
            }
            break;
        }
        if cur.bump() != Some(')') {
            return Err("expected )".to_string());
        }
        if !cur.eat_word("VALUES") {
            return Err("expected VALUES".to_string());
        }
        cur.skip_ws();
        if cur.bump() != Some('(') {
            return Err("expected (".to_string());
        }
        let mut count = 0_usize;
        loop {
            cur.skip_ws();
            if cur.bump() != Some('?') {
                return Err("expected placeholder".to_string());
            }
            count += 1;
            cur.skip_ws();
            if cur.peek() == Some(',') {
                cur.bump();
                continue;
            }
            break;
        }
        if cur.bump() != Some(')') {
            return Err("expected )".to_string());
        }
        if count != columns.len() {
            return Err("insert column count does not match placeholders".to_string());
        }
        cur.skip_ws();
        if cur.peek().is_some() {
            return Err("unexpected sql tail".to_string());
        }
        Ok(Stmt::Insert { table, columns })
    }

    fn parse_select(cur: &mut Cur<'_>) -> Result<SelectQuery, String> {
        cur.skip_ws();
        let count = if cur.eat_word("COUNT") {
            cur.skip_ws();
            if cur.bump() != Some('(')
                || {
                    cur.skip_ws();
                    cur.bump() != Some('*')
                }
                || {
                    cur.skip_ws();
                    cur.bump() != Some(')')
                }
            {
                return Err("expected COUNT(*)".to_string());
            }
            true
        } else {
            false
        };
        let mut columns = Vec::new();
        if !count {
            loop {
                columns.push(cur.quoted()?);
                cur.skip_ws();
                if cur.peek() == Some(',') {
                    cur.bump();
                    continue;
                }
                break;
            }
        }
        if !cur.eat_word("FROM") {
            return Err("expected FROM".to_string());
        }
        let table = cur.quoted()?;
        let pred = if cur.eat_word("WHERE") {
            Some(parse_or(cur)?)
        } else {
            None
        };
        let mut order = Vec::new();
        if cur.eat_word("ORDER") {
            if !cur.eat_word("BY") {
                return Err("expected BY".to_string());
            }
            loop {
                let field = cur.quoted()?;
                let descending = if cur.eat_word("DESC") {
                    true
                } else {
                    let _ = cur.eat_word("ASC");
                    false
                };
                order.push((field, descending));
                cur.skip_ws();
                if cur.peek() == Some(',') {
                    cur.bump();
                    continue;
                }
                break;
            }
        }
        let limit = if cur.eat_word("LIMIT") {
            cur.skip_ws();
            if cur.bump() != Some('?') {
                return Err("expected limit placeholder".to_string());
            }
            true
        } else {
            false
        };
        let offset = if cur.eat_word("OFFSET") {
            cur.skip_ws();
            if cur.bump() != Some('?') {
                return Err("expected offset placeholder".to_string());
            }
            true
        } else {
            false
        };
        cur.skip_ws();
        if cur.peek().is_some() {
            return Err("unexpected sql tail".to_string());
        }
        Ok(SelectQuery {
            columns,
            count,
            table,
            pred,
            order,
            limit,
            offset,
        })
    }

    fn parse_or(cur: &mut Cur<'_>) -> Result<Pred, String> {
        let mut parts = vec![parse_and(cur)?];
        while cur.eat_word("OR") {
            parts.push(parse_and(cur)?);
        }
        flatten(parts, Pred::Or)
    }

    fn parse_and(cur: &mut Cur<'_>) -> Result<Pred, String> {
        let mut parts = vec![parse_atom(cur)?];
        while cur.eat_word("AND") {
            parts.push(parse_atom(cur)?);
        }
        flatten(parts, Pred::And)
    }

    fn flatten(mut parts: Vec<Pred>, combine: fn(Vec<Pred>) -> Pred) -> Result<Pred, String> {
        if parts.len() == 1 {
            return parts.pop().ok_or_else(|| "empty condition".to_string());
        }
        Ok(combine(parts))
    }

    fn parse_atom(cur: &mut Cur<'_>) -> Result<Pred, String> {
        cur.skip_ws();
        if cur.peek() == Some('(') {
            cur.bump();
            let inner = parse_or(cur)?;
            cur.skip_ws();
            if cur.bump() != Some(')') {
                return Err("expected )".to_string());
            }
            return Ok(inner);
        }
        let field = cur.quoted()?;
        cur.skip_ws();
        if cur.eat_word("IS") {
            let negated = cur.eat_word("NOT");
            if !cur.eat_word("NULL") {
                return Err("expected NULL".to_string());
            }
            return Ok(Pred::IsNull { field, negated });
        }
        if cur.eat_word("LIKE") || cur.eat_word("IN") || cur.eat_word("BETWEEN") || cur.eat_word("NOT") {
            return Err("this condition is not implemented".to_string());
        }
        let op = parse_op(cur)?;
        cur.skip_ws();
        if cur.bump() != Some('?') {
            return Err("expected placeholder".to_string());
        }
        Ok(Pred::Cmp { field, op })
    }

    fn parse_op(cur: &mut Cur<'_>) -> Result<Op, String> {
        cur.skip_ws();
        let Some(first) = cur.bump() else {
            return Err("expected an operator".to_string());
        };
        let second = cur.peek();
        let op = match (first, second) {
            ('<', Some('>')) => Op::Ne,
            ('<', Some('=')) => Op::Le,
            ('>', Some('=')) => Op::Ge,
            ('=', _) => return Ok(Op::Eq),
            ('<', _) => return Ok(Op::Lt),
            ('>', _) => return Ok(Op::Gt),
            _ => return Err("expected an operator".to_string()),
        };
        cur.bump();
        Ok(op)
    }
}
