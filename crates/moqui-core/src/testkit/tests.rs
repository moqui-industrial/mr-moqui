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

//! The harness on the in-memory database: what a spec file can say, and what a run reports.

use super::run::{run_spec, Outcome, RunOptions, SpecFile, SpecResult};
use crate::context::ExecutionContextFactory;

fn run(source: &str) -> SpecResult {
    run_with(source, &RunOptions::default())
}

fn run_with(source: &str, options: &RunOptions) -> SpecResult {
    crate::script::clear_context_stack();
    let context = ExecutionContextFactory::test_context();
    context.factory().set_entity(crate::testing::open());
    let _entered = context.enter();
    let spec = SpecFile {
        name: "Sample".to_string(),
        display: "test/Sample.spec.js".to_string(),
        source: source.to_string(),
    };
    run_spec(&spec, options, &mut |_| {})
}

fn outcomes(result: &SpecResult) -> Vec<(String, &'static str)> {
    result
        .cases
        .iter()
        .map(|case| {
            let kind = match &case.outcome {
                Outcome::Passed => "passed",
                Outcome::Failed(_) => "failed",
                Outcome::Ignored(_) => "ignored",
                Outcome::Skipped(_) => "skipped",
            };
            (case.name.clone(), kind)
        })
        .collect()
}

fn failure(result: &SpecResult, index: usize) -> &super::run::Failure {
    match &result.cases[index].outcome {
        Outcome::Failed(failure) => failure,
        other => panic!("case {index} did not fail: {other:?}"),
    }
}

#[test]
fn features_share_file_state_and_pass_values_from_when_to_then() {
    let result = run(r#"
let shared = 0
const order = []
spec({
    setupSpec() { order.push("setupSpec") },
    setup() { order.push("setup") },
    cleanup() { order.push("cleanup") },
    cleanupSpec() { if (order.join() !== "setupSpec,setup,cleanup,setup,cleanup") throw new Error(order.join()) },
})
feature("first", {
    when() { shared = 2; return { total: toDecimal("16.990"), list: [1, toDecimal("2.0")] } },
    then({ total, list }) {
        total == 16.99
        total > 10
        shared == 2
        list == [1, 2]
        list != []
        !""
    },
})
feature("second", {
    given() { return { a: 1 } },
    expect: ({ a }) => a + shared == "3",
})
"#);
    assert!(result.setup_failure.is_none(), "{:?}", result.setup_failure);
    assert!(result.cleanup_failure.is_none(), "{:?}", result.cleanup_failure);
    assert_eq!(
        outcomes(&result),
        vec![("first".to_string(), "passed"), ("second".to_string(), "passed")]
    );
}

#[test]
fn a_false_condition_fails_with_a_diagram_and_its_line() {
    let result = run(r#"spec({})
feature("prices", {
    when() { return { priceMap: { price: toDecimal("16.98"), uom: "USD" } } },
    then({ priceMap }) {
        priceMap.uom == "USD"
        priceMap.price == 16.99
    },
})
"#);
    let failure = failure(&result, 0);
    assert_eq!(failure.kind, "assertion");
    assert_eq!(failure.block, "then");
    assert_eq!(failure.location.as_deref(), Some("test/Sample.spec.js:6"));
    assert_eq!(
        failure.message,
        "Condition not satisfied:\n\npriceMap.price == 16.99\n|        |     |\n|        16.98 false\n{\"price\":\"16.98\",\"uom\":\"USD\"}"
    );
}

#[test]
fn groovy_truth_and_void_calls() {
    let result = run(r#"
feature("empty list is false", { expect() { !list }, where: [{ list: [] }] })
feature("void call passes", { expect() { (function () {})() } })
feature("empty list fails", { expect() { [] } })
feature("zero decimal fails", { expect() { toDecimal("0.00") } })
"#);
    assert_eq!(
        outcomes(&result),
        vec![
            ("empty list is false".to_string(), "passed"),
            ("void call passes".to_string(), "passed"),
            ("empty list fails".to_string(), "failed"),
            ("zero decimal fails".to_string(), "failed"),
        ]
    );
}

#[test]
fn assert_checks_a_condition_in_a_loop() {
    let result =
        run("feature('loop', {\n  expect() {\n    for (const n of [1, 2, 3]) assert(n < 3, `n is ${n}`)\n  },\n})\n");
    let failure = failure(&result, 0);
    assert_eq!(failure.kind, "assertion");
    assert_eq!(failure.message, "Assertion failed: n is 3\n\nvalue: false");
    assert_eq!(
        failure.location.as_deref(),
        Some("test/Sample.spec.js:3"),
        "{failure:?}"
    );
}

#[test]
fn an_error_names_the_block_and_the_spec_line() {
    let result = run("feature('throws', {\n  when() {\n    const x = null\n    return { y: x.field }\n  },\n  then({ y }) { y == 1 },\n})\n");
    let failure = failure(&result, 0);
    assert_eq!(failure.kind, "error");
    assert_eq!(failure.block, "when");
    assert_eq!(
        failure.location.as_deref(),
        Some("test/Sample.spec.js:4"),
        "{failure:?}"
    );
    assert!(failure.message.starts_with("TypeError: "), "{}", failure.message);
}

#[test]
fn where_rows_unroll_and_the_filter_picks_features() {
    let source = r#"
feature("row #a plus #b", {
    where: [{ a: 1, b: 2, sum: 3 }, { a: "x", b: 2, sum: 5 }],
    expect: ({ a, b, sum }) => a + b == sum,
})
feature("other", { expect() { true } })
"#;
    let result = run(source);
    assert_eq!(
        outcomes(&result),
        vec![
            ("row 1 plus 2".to_string(), "passed"),
            ("row \"x\" plus 2".to_string(), "failed"),
            ("other".to_string(), "passed"),
        ]
    );
    let filtered = run_with(
        source,
        &RunOptions {
            feature_filter: Some("other".to_string()),
        },
    );
    assert_eq!(outcomes(&filtered), vec![("other".to_string(), "passed")]);
}

#[test]
fn stepwise_skips_after_a_failure_and_ignore_is_reported() {
    let result = run(r#"
spec({ stepwise: true })
feature("a", { ignore: "not yet", expect() { false } })
feature("b", { expect() { 1 == 2 } })
feature("c", { expect() { true } })
"#);
    assert_eq!(
        outcomes(&result),
        vec![
            ("a".to_string(), "ignored"),
            ("b".to_string(), "failed"),
            ("c".to_string(), "skipped")
        ]
    );
}

#[test]
fn message_errors_fail_a_feature_unless_allowed() {
    let result = run(r#"
feature("left an error", { expect() { ec.message.addError("left behind") } })
feature("allowed", { allowMessageErrors: true, expect() { ec.message.addError("fine") } })
feature("next starts clean", { expect() { !ec.message.hasError() } })
feature("service failure throws", { when() { call("no.such.Service.do#Nothing", {}) }, then() { true } })
"#);
    assert_eq!(
        outcomes(&result),
        vec![
            ("left an error".to_string(), "failed"),
            ("allowed".to_string(), "passed"),
            ("next starts clean".to_string(), "passed"),
            ("service failure throws".to_string(), "failed"),
        ]
    );
    assert_eq!(failure(&result, 0).kind, "messages");
    assert_eq!(failure(&result, 0).details, vec!["left behind".to_string()]);
    let thrown = failure(&result, 3);
    assert!(
        thrown
            .message
            .starts_with("Error: service no.such.Service.do#Nothing failed"),
        "{}",
        thrown.message
    );
}

#[test]
fn registration_errors_fail_the_spec() {
    let unrewritten = run("const make = () => ({ then() { 1 == 2 }, when() {} })\nfeature('helper built', make())\n");
    let failure = unrewritten.setup_failure.expect("registration failure");
    assert_eq!(failure.kind, "registration");
    assert!(failure.message.contains("was not rewritten"), "{}", failure.message);
    let misspelled = run("feature('typo', { expcet() { true } })\n");
    assert!(misspelled
        .setup_failure
        .expect("registration failure")
        .message
        .contains("unknown field expcet"));
    let syntax = run("feature('bad', { expect() { 1 == } })\n");
    assert!(syntax.setup_failure.expect("syntax").message.contains("line 1"));
}

#[test]
fn setup_spec_failure_skips_the_features_and_cleanup_still_runs() {
    let result = run(r#"
let cleaned = false
spec({ setupSpec() { throw new Error("no fixture") }, cleanupSpec() { cleaned = true; throw new Error("cleaned") } })
feature("a", { expect() { true } })
"#);
    assert!(result.setup_failure.as_ref().unwrap().message.contains("no fixture"));
    assert_eq!(result.cleanup_failure.as_ref().unwrap().message, "Error: cleaned");
    assert_eq!(outcomes(&result), vec![("a".to_string(), "skipped")]);
}

#[test]
fn temp_sequences_effective_time_and_check_data() {
    let result = run(r#"
const effectiveTime = 1265198400000
spec({
    tempSequences: { start: 55500, banks: { "moqui.basic.Enumeration": 10 } },
    setupSpec() { ec.user.setEffectiveTime(effectiveTime) },
    setup() { ec.artifactExecution.disableAuthz() },
    cleanup() { ec.artifactExecution.enableAuthz() },
})
feature("pinned", {
    when() {
        const e = ec.entity.makeValue("moqui.basic.Enumeration")
        e.enumTypeId = "T1"; e.description = "one"
        e.setSequencedIdPrimary(); e.create()
        const errs = checkData(`<entity-facade-xml>
            <moqui.basic.Enumeration enumId="55500" enumTypeId="T1" description="one"/></entity-facade-xml>`)
        const bad = checkData(`<entity-facade-xml>
            <moqui.basic.Enumeration enumId="55500" description="two"/></entity-facade-xml>`)
        return { id: e.enumId, errs, bad, now: ec.user.nowTimestamp }
    },
    then({ id, errs, bad, now }) {
        id == "55500"
        errs.length == 0
        bad.length == 1
        now == effectiveTime
    },
})
"#);
    assert!(result.setup_failure.is_none(), "{:?}", result.setup_failure);
    assert_eq!(
        outcomes(&result),
        vec![("pinned".to_string(), "passed")],
        "{:?}",
        result.cases
    );
}

#[test]
fn data_check_messages_are_in_a_failure() {
    let result = run(r#"
feature("check", {
    when() { return { errs: checkData(`<entity-facade-xml><moqui.basic.Enumeration enumId="NOPE"/></entity-facade-xml>`) } },
    then({ errs }) { errs.length == 0 },
})
"#);
    let failure = failure(&result, 0);
    assert_eq!(
        failure.details,
        vec!["data check: Entity moqui.basic.Enumeration record not found for primary key {enumId=NOPE}".to_string()]
    );
}

#[test]
fn parallel_runs_a_function_on_threads_with_their_own_contexts() {
    let result = run(r#"
feature("threads", {
    when() {
        return { out: parallel(3, (n, args) => ({ n, base: args.base, user: ec.user.username }), { base: 10 }, { transaction: false }) }
    },
    then({ out }) {
        out.length == 3
        out.map(o => o.n + o.base).join() == "10,11,12"
        out[0].user == null
    },
})
feature("a failing thread fails the feature", {
    expect() { parallel(2, (n) => { if (n === 1) throw new Error("thread one") }, null, { transaction: false }) },
})
"#);
    assert_eq!(
        outcomes(&result),
        vec![
            ("threads".to_string(), "passed"),
            ("a failing thread fails the feature".to_string(), "failed"),
        ],
        "{:?}",
        result.cases
    );
    assert!(
        failure(&result, 1).message.contains("thread 1: Error: thread one"),
        "{}",
        failure(&result, 1).message
    );
}
