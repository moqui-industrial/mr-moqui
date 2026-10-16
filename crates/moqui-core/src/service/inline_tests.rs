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

//! The inline executor: async work drains on the calling thread, `callFuture()` runs at once.

use std::collections::BTreeMap;
use std::path::PathBuf;

use moqui_lib::object::Value;

use super::pool::ServicePools;
use crate::context::{BootOptions, ExecutionContext, ExecutionContextFactory, Executor};
use crate::entity::EntityFacade;

const SERVICES: &str = r#"<services>
    <service verb="inline" noun="Mark" authenticate="false">
        <in-parameters><parameter name="id"/><parameter name="after"/></in-parameters>
        <actions>
            <entity-find-one entity-name="moqui.test.TestEntity" value-field="prior">
                <field-map field-name="testId" from="after"/>
            </entity-find-one>
            <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
            <script>made.set("testId", id); made.set("testMedium", prior ? "seen" : "missing");</script>
            <entity-create value-field="made"/>
        </actions>
    </service>
    <service verb="inline" noun="Echo" authenticate="false" transaction="ignore">
        <in-parameters><parameter name="note"/></in-parameters>
        <out-parameters><parameter name="note"/></out-parameters>
    </service>
    <service verb="inline" noun="MarkOrFail" authenticate="false">
        <in-parameters><parameter name="id"/></in-parameters>
        <actions>
            <if condition="id == 'BAD'"><return error="true" message="bad row"/></if>
            <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
            <script>made.set("testId", id); made.set("testMedium", "made");</script>
            <entity-create value-field="made"/>
        </actions>
    </service>
    <service verb="inline" noun="IgnoreFail" authenticate="false" transaction="ignore">
        <actions><return error="true" message="ignore failed"/></actions>
    </service>
    <service verb="inline" noun="NoCache" authenticate="false" no-tx-cache="true">
        <in-parameters><parameter name="id"/></in-parameters>
        <out-parameters><parameter name="found"/></out-parameters>
        <actions>
            <script>found = ec.entity.find("moqui.test.TestEntity").condition("testId", id).useCache(false).one() != null ? "Y" : "N"</script>
        </actions>
    </service>
</services>"#;

fn inline_context() -> (EntityFacade, ExecutionContext) {
    // A file of its own for each test: tests run at once, and one writing the file another reads is a parse error.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("moqui-svc-inline-executor-{}-{next}.xml", std::process::id()));
    std::fs::write(&path, SERVICES).unwrap();
    let facade = crate::testing::open();
    let ctx = ExecutionContextFactory::test_context_with("", ServicePools::inline());
    ctx.factory().set_entity(facade.clone());
    {
        let _entered = ctx.enter();
        let failures = crate::service::lint(std::slice::from_ref(&path)).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
    }
    (facade, ctx)
}

fn mark(id: &str, after: &str) -> BTreeMap<String, Value> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), Value::string(id));
    params.insert("after".into(), Value::string(after));
    params
}

fn medium(facade: &EntityFacade, id: &str) -> Option<String> {
    facade
        .find("moqui.test.TestEntity")
        .unwrap()
        .eq("testId", Value::string(id))
        .one()
        .unwrap()
        .and_then(|row| row.get("testMedium").and_then(Value::as_str).map(str::to_string))
}

#[test]
fn async_calls_wait_for_run_pending_and_run_in_order() {
    let (facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    super::async_call("inline#Mark", &mark("IA", "none")).unwrap();
    super::async_call("inline#Mark", &mark("IB", "IA")).unwrap();
    assert_eq!(ctx.factory().service_pools().pending(), 2);
    assert_eq!(medium(&facade, "IA"), None, "queued work ran early");
    assert_eq!(ctx.factory().run_pending(), 2);
    assert_eq!(medium(&facade, "IA").as_deref(), Some("missing"));
    assert_eq!(medium(&facade, "IB").as_deref(), Some("seen"));
    assert_eq!(ctx.factory().run_pending(), 0);
}

#[test]
fn leaving_the_outermost_context_drains() {
    let (facade, ctx) = inline_context();
    {
        let _entered = ctx.enter();
        let _authz = ctx.disable_authz();
        super::async_call("inline#Mark", &mark("IC", "none")).unwrap();
        {
            let _inner = ctx.enter();
        }
        assert_eq!(ctx.factory().service_pools().pending(), 1, "an inner guard drained");
    }
    assert_eq!(ctx.factory().service_pools().pending(), 0);
    assert_eq!(medium(&facade, "IC").as_deref(), Some("missing"));
}

#[test]
fn run_pending_waits_while_a_transaction_is_in_place() {
    let (facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let tx = facade.transactions_for("moqui.test.TestEntity").unwrap();
    assert!(tx.begin(None).unwrap());
    super::async_call("inline#Mark", &mark("ID", "none")).unwrap();
    assert_eq!(ctx.factory().run_pending(), 0);
    tx.commit().unwrap();
    assert_eq!(ctx.factory().run_pending(), 1);
    assert_eq!(medium(&facade, "ID").as_deref(), Some("missing"));
}

#[test]
fn a_failed_queued_call_is_logged_and_the_rest_run() {
    let (facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    super::async_call("inline#NoSuchService", &BTreeMap::new()).unwrap();
    super::async_call("inline#Mark", &mark("IE", "none")).unwrap();
    assert_eq!(ctx.factory().run_pending(), 2);
    assert_eq!(medium(&facade, "IE").as_deref(), Some("missing"));
}

#[test]
fn call_future_and_start_future_run_at_once() {
    let (_facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let mut params = BTreeMap::new();
    params.insert("note".into(), Value::string("hello"));
    let out = super::call_future("inline#Echo", &params).unwrap();
    assert_eq!(out.get("note").and_then(Value::as_str), Some("hello"));
    let mut future = super::start_future("inline#Echo", &params).unwrap();
    let out = future.get(None).unwrap().expect("settled");
    assert_eq!(out.get("note").and_then(Value::as_str), Some("hello"));
    assert!(super::call_future("inline#NoSuchService", &params).is_err());
    assert_eq!(ctx.factory().service_pools().pending(), 0);
}

#[test]
fn an_inline_boot_starts_no_pool_and_no_deadline_timer() {
    let runtime = std::env::temp_dir().join(format!("moqui-inline-boot-{}", std::process::id()));
    std::fs::create_dir_all(&runtime).unwrap();
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
    let factory = {
        let _lock = crate::user::session::cookie_property_lock();
        ExecutionContextFactory::boot_with_options(
            &runtime,
            &framework,
            BootOptions {
                executor: Executor::Inline,
                ..Default::default()
            },
        )
        .expect("boot")
    };
    assert_eq!(factory.executor(), Executor::Inline);
    assert!(factory.service_pools().is_inline());
    assert_eq!(factory.service_pools().sizes(), (0, 0));
    factory.start_request_pool().unwrap();
    assert!(factory
        .request_pool()
        .try_submit(std::sync::Arc::clone(&factory), || Ok(()))
        .is_err());
    assert!(!crate::transaction::single_threaded() || cfg!(target_os = "emscripten"));
    let ctx = factory.new_context();
    {
        let _entered = ctx.enter();
        assert!(crate::transaction::single_threaded(), "the deadline timer would start");
    }
    drop(ctx);
    drop(factory);
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn an_ignore_service_runs_in_the_callers_transaction_and_its_error_marks_it() {
    let (facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let tx = facade.transactions_for("moqui.test.TestEntity").unwrap();
    assert!(tx.begin(None).unwrap());
    super::sync("inline#IgnoreFail", &BTreeMap::new()).unwrap();
    assert!(tx.in_transaction(), "ignore left the caller's transaction in place");
    let err = tx.commit().expect_err("the caller's transaction is rollback-only");
    assert!(err.to_string().contains("inline#IgnoreFail"), "{err}");
    ctx.message().clear();
}

#[test]
fn a_no_tx_cache_service_sees_the_rows_the_caller_held() {
    let (facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let tx = facade.transactions_for("moqui.test.TestEntity").unwrap();
    tx.run(crate::transaction::TransactionAttribute::Cache, None, || {
        super::sync("inline#Mark", &mark("NC", "none"))?;
        assert!(tx.transaction_cache_active());
        let mut id = BTreeMap::new();
        id.insert("id".into(), Value::string("NC"));
        let out = super::sync("inline#NoCache", &id)?;
        assert_eq!(out.get("found"), Some(&Value::string("Y")));
        assert!(!tx.transaction_cache_active(), "no-tx-cache stops holding");
        Ok(())
    })
    .unwrap();
    assert_eq!(medium(&facade, "NC").as_deref(), Some("missing"));
}

#[test]
fn a_builder_changes_in_place_and_an_in_map_of_the_context_passes_it() {
    let (_facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    crate::script::run_actions(
        r#"<actions>
            <set field="note" value="from context"/>
            <service-call name="inline#Echo" in-map="context" out-map="viaContext"/>
            <service-call name="inline#Echo" in-map="true" out-map="viaTrue">
                <field-map field-name="note" value="from field map"/>
            </service-call>
            <script>(function () {
                const builder = ec.service.sync().name("inline#Echo");
                sameBuilder = builder.parameter("note", "chained") === builder;
                chained = builder.call().note;
                fromObject = ec.service.sync().name("inline#Echo").parameters({note: "from object"}).call().note;
            })()</script>
        </actions>"#,
    )
    .unwrap();
    for check in [
        "viaContext.note === 'from context'",
        "viaTrue.note === 'from field map'",
        "sameBuilder === true",
        "chained === 'chained'",
        "fromObject === 'from object'",
    ] {
        assert!(crate::script::eval_condition(check).unwrap(), "{check}");
    }
    crate::script::clear_context_stack();
}

#[test]
fn a_multi_call_runs_each_row_in_one_transaction() {
    let (facade, ctx) = inline_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    crate::script::run_script(
        r#"ec.service.sync().name("inline#Mark").parameters({id_0: "MU0", after_0: "none", id_1: "MU1", after_1: "MU0",
            id_2: "MU2", after_2: "none", _useRowSubmit_2: "true"}).multi(true).call()"#,
    )
    .unwrap();
    assert_eq!(medium(&facade, "MU0").as_deref(), Some("missing"));
    assert_eq!(
        medium(&facade, "MU1").as_deref(),
        Some("seen"),
        "row 1 sees row 0 in the same transaction"
    );
    assert_eq!(
        medium(&facade, "MU2"),
        None,
        "a row that uses row submit without _rowSubmit_2 is skipped"
    );
    // An error in a row stops the rows and rolls back the ones before it.
    crate::script::run_script(
        r#"ec.service.sync().name("inline#MarkOrFail").parameters({id_0: "MV0", id_1: "BAD", id_2: "MV2"}).multi(true).call()"#,
    )
    .unwrap();
    assert!(
        ctx.message().errors().iter().any(|error| error.contains("bad row")),
        "{:?}",
        ctx.message().errors()
    );
    assert_eq!(medium(&facade, "MV0"), None, "row 0 rolled back with the failed row 1");
    assert_eq!(medium(&facade, "MV2"), None, "the rows after the error do not run");
    ctx.message().clear();
}
