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

use std::collections::BTreeMap;

use moqui_lib::object::Value;

use super::{parameter_string, ArtifactHits, Hit, HitContext};

fn conf() -> &'static str {
    r#"<moqui-conf><server-stats bin-length-seconds="60">
            <artifact-stats type="AT_XML_SCREEN" persist-bin="true" persist-hit="true"/>
            <artifact-stats type="AT_SERVICE" persist-bin="true" persist-hit="false"/>
            <artifact-stats type="AT_ENTITY" persist-bin="false"/>
        </server-stats></moqui-conf>"#
}

fn hit<'a>(artifact_type: &'a str, name: &'a str, start_ms: i64, running_ms: f64) -> Hit<'a> {
    Hit {
        artifact_type,
        sub_type: None,
        name,
        parameters: None,
        start_ms,
        running_ms,
        output_size: None,
    }
}

#[test]
fn bins_follow_the_conf_and_advance_after_the_bin_length() {
    let hits = ArtifactHits::from_conf(conf());
    assert_eq!(hits.bin_length_ms, 60_000);
    assert!(hits.counts("AT_SERVICE") && !hits.counts("AT_ENTITY") && !hits.counts("AT_LLM"));
    hits.count(&hit("AT_SERVICE", "a.b#c", 1_000, 4.0), HitContext::default);
    hits.count(&hit("AT_SERVICE", "a.b#c", 2_000, 2.0), HitContext::default);
    hits.count(
        &hit("AT_ENTITY", "moqui.basic.Enumeration", 2_000, 2.0),
        HitContext::default,
    );
    hits.count(
        &hit("AT_XML_SCREEN", "component://x.xml", 3_000, 5.0),
        HitContext::default,
    );
    // A hit after the bin length closes the old bin at its end and starts another.
    hits.count(&hit("AT_SERVICE", "a.b#c", 70_000, 1.0), HitContext::default);
    let state = hits.state.lock().unwrap();
    assert_eq!(state.closed.len(), 1);
    let closed = &state.closed[0];
    assert_eq!(
        (closed.bin.start_ms, closed.end_ms, closed.bin.hit_count),
        (1_000, 61_000, 2)
    );
    assert_eq!(
        (closed.bin.min_time, closed.bin.max_time, closed.bin.total_time),
        (2.0, 4.0, 6.0)
    );
    assert_eq!(state.stats.len(), 2);
    let service = &state.stats["a.b#c"];
    assert_eq!(
        (service.hit_count, service.cur_bin.as_ref().unwrap().start_ms),
        (3, 70_000)
    );
    // Only the screen has persist-hit.
    assert_eq!(state.hits.len(), 1);
    assert_eq!(state.hits[0].name, "component://x.xml");
}

#[test]
fn count_asks_for_a_flush_at_the_threshold_and_a_flush_in_time_drops_nothing() {
    use super::{FLUSH_QUEUED_HITS, MAX_QUEUED_HITS};

    let hits = ArtifactHits::from_conf(conf());
    let screen = |index: i64| hit("AT_XML_SCREEN", "component://x.xml", index, 1.0);
    for index in 0..(FLUSH_QUEUED_HITS - 1) {
        assert!(!hits.count(&screen(index as i64), HitContext::default));
    }
    assert!(hits.count(&screen(0), HitContext::default), "threshold reached");
    // A type that is not counted never asks.
    assert!(!hits.count(&hit("AT_LLM", "m", 0, 1.0), HitContext::default));

    // The caller flushes when asked, as `spawn_flush` does with one flush at a time: nothing is dropped
    // though three times the limit is counted with no scheduler tick.
    let mut flushed = 0;
    for index in 0..(MAX_QUEUED_HITS * 3) {
        if hits.count(&screen(index as i64), HitContext::default) && hits.begin_flush() {
            flushed += hits.take(false).0.len();
            hits.flushing.store(false, std::sync::atomic::Ordering::Release);
        }
    }
    let state = hits.state.lock().unwrap();
    assert_eq!(state.dropped, 0);
    assert_eq!(flushed + state.hits.len(), FLUSH_QUEUED_HITS + MAX_QUEUED_HITS * 3);
}

#[test]
fn current_bins_are_taken_in_java_hash_map_order() {
    use super::java_hash_map_order;

    let order = |keys: &[&str]| java_hash_map_order(keys.iter().enumerate().map(|(index, key)| (*key, index as u64)));
    // Buckets by String.hashCode: "c" 99, "a" 97, "b" 98, whatever the insertion order.
    assert_eq!(order(&["c", "a", "b"]), ["a", "b", "c"]);
    // "BB" and "Aa" share hash 2112: one bucket, in insertion order.
    assert_eq!(order(&["BB", "Aa"]), ["BB", "Aa"]);
    assert_eq!(order(&["Aa", "BB"]), ["Aa", "BB"]);
    // 13 keys pass the 3/4 threshold of 16, so 32 buckets: "q" (113) is bucket 1 of 16 but 17 of 32.
    let letters = ["q", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"];
    assert_eq!(order(&letters)[0], "q");
    let mut thirteen = letters.to_vec();
    thirteen.push("l");
    assert_eq!(order(&thirteen).last().map(String::as_str), Some("q"));
    // Java wrote these first, in this order, at the end of an empty-db load of 85 bins (128 buckets). Inserted
    // in Java's order, EnumerationType and UomConversion share bucket 4.
    let mut keys = vec![
        "moqui.basic.EnumerationTyperefresh".to_string(),
        "moqui.security.ArtifactTarpitrefresh".to_string(),
        "moqui.basic.UomConversionrefresh".to_string(),
    ];
    keys.extend((0..82).map(|index| format!("pad{index}")));
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let ordered: Vec<String> = order(&keys)
        .into_iter()
        .filter(|key| key.starts_with("moqui."))
        .collect();
    assert_eq!(
        ordered,
        [
            "moqui.security.ArtifactTarpitrefresh",
            "moqui.basic.EnumerationTyperefresh",
            "moqui.basic.UomConversionrefresh",
        ]
    );
}

#[test]
fn rust_only_bins_are_taken_after_the_java_order() {
    let hits = ArtifactHits::from_conf(conf());
    let fk_check = Hit {
        sub_type: Some(crate::entity::RustOnlyEntityHits::FK_CHECK),
        ..hit("AT_XML_SCREEN", "a", 1_000, 1.0)
    };
    hits.count(&fk_check, HitContext::default);
    for name in ["c", "a", "b"] {
        hits.count(&hit("AT_XML_SCREEN", name, 1_000, 1.0), HitContext::default);
    }
    let (_, bins) = hits.take(true);
    let taken: Vec<(&str, Option<&str>)> = bins
        .iter()
        .map(|bin| (bin.name.as_str(), bin.sub_type.as_deref()))
        .collect();
    assert_eq!(taken, [("a", None), ("b", None), ("c", None), ("a", Some("fk-check"))]);
}

#[test]
fn only_one_background_flush_is_claimed_at_a_time() {
    let hits = ArtifactHits::from_conf(conf());
    assert!(hits.begin_flush());
    assert!(!hits.begin_flush());
    hits.flushing.store(false, std::sync::atomic::Ordering::Release);
    assert!(hits.begin_flush());
}

#[test]
fn without_a_flush_the_limit_still_drops_and_counts() {
    use super::MAX_QUEUED_HITS;

    let hits = ArtifactHits::from_conf(conf());
    for index in 0..(MAX_QUEUED_HITS + 5) {
        hits.count(
            &hit("AT_XML_SCREEN", "component://x.xml", index as i64, 1.0),
            HitContext::default,
        );
    }
    let state = hits.state.lock().unwrap();
    assert_eq!((state.hits.len(), state.dropped), (MAX_QUEUED_HITS, 5));
}

#[test]
fn a_hit_far_above_the_average_is_slow_after_fifty() {
    let hits = ArtifactHits::from_conf(conf());
    for index in 0..60 {
        hits.count(
            &hit("AT_SERVICE", "s", index, 10.0 + (index % 3) as f64),
            HitContext::default,
        );
    }
    // Slow and over the user impact time: written though AT_SERVICE does not persist hits.
    hits.count(&hit("AT_SERVICE", "s", 61, 1500.0), HitContext::default);
    let state = hits.state.lock().unwrap();
    assert_eq!(state.stats["s"].cur_bin.as_ref().unwrap().slow_hit_count, 1);
    assert_eq!(state.hits.len(), 1);
    assert!(state.hits[0].is_slow);
}

#[test]
fn the_parameter_string_skips_passwords_maps_and_empty_values() {
    let mut parameters = BTreeMap::new();
    parameters.insert("a".to_string(), Value::string("x".repeat(100)));
    parameters.insert("newPassword".to_string(), Value::string("no"));
    parameters.insert("password".to_string(), Value::string("no"));
    parameters.insert("empty".to_string(), Value::string(""));
    parameters.insert("list".to_string(), Value::List(vec![Value::I64(1)]));
    parameters.insert("n".to_string(), Value::I64(5));
    for secret in [
        "code",
        "moquiSessionToken",
        "sql",
        "sharedSecret",
        "api_key",
        "login_key",
        "clientSecret",
        "refreshToken",
        "AUTH_TOKEN",
        "privateApiKey",
    ] {
        parameters.insert(secret.to_string(), Value::string("no"));
    }
    // Ordinary ids are kept.
    parameters.insert("primaryKeyId".to_string(), Value::string("k"));
    parameters.insert("monkeyId".to_string(), Value::string("m"));
    parameters.insert("codeId".to_string(), Value::string("c"));
    let text = parameter_string(&parameters).unwrap();
    assert_eq!(
        text,
        format!("a={}, codeId=c, monkeyId=m, n=5, primaryKeyId=k", "x".repeat(80))
    );
}

#[test]
fn queued_hits_and_new_artifacts_stop_at_the_limits_until_a_flush_takes_them() {
    let hits = ArtifactHits::from_conf(conf());
    for index in 0..super::MAX_QUEUED_HITS + 5 {
        hits.count(
            &hit("AT_XML_SCREEN", "component://a/screen/A.xml", index as i64, 1.0),
            HitContext::default,
        );
    }
    {
        let state = hits.state.lock().unwrap();
        assert_eq!(state.hits.len(), super::MAX_QUEUED_HITS);
        assert_eq!(state.dropped, 5);
    }
    let (taken, _) = hits.take(false);
    assert_eq!(taken.len(), super::MAX_QUEUED_HITS);
    assert_eq!(hits.state.lock().unwrap().dropped, 0);
    hits.count(
        &hit("AT_XML_SCREEN", "component://a/screen/A.xml", 0, 1.0),
        HitContext::default,
    );
    assert_eq!(hits.state.lock().unwrap().hits.len(), 1);

    let names: Vec<String> = (0..super::MAX_STATS + 3).map(|index| format!("svc.{index}")).collect();
    for name in &names {
        hits.count(&hit("AT_SERVICE", name, 0, 1.0), HitContext::default);
    }
    let state = hits.state.lock().unwrap();
    // The screen artifact, then services up to the limit.
    assert_eq!(state.stats.len(), super::MAX_STATS);
    assert_eq!(state.dropped, 4);
}

/// A world with the conf's server stats, a pool to tick, screens, and a service.
struct World {
    context: crate::context::ExecutionContext,
    facade: crate::entity::EntityFacade,
    tape: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    dir: std::path::PathBuf,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const SCREEN: &str = r#"<screen require-authentication="false">
        <transition name="doIt"><default-response url="."/></transition>
        <widgets><label text="tracked"/></widgets></screen>"#;
const UNTRACKED: &str = r#"<screen require-authentication="false" track-artifact-hit="false">
        <widgets><label text="untracked"/></widgets></screen>"#;
const SERVICES: &str = r#"<services>
        <service verb="find" noun="Hits" authenticate="false">
            <in-parameters><parameter name="enumTypeId"/><parameter name="password"/></in-parameters>
            <actions><entity-find entity-name="moqui.basic.Enumeration" list="found">
                <econdition field-name="enumTypeId"/></entity-find></actions>
        </service></services>"#;

fn world() -> World {
    let conf = r#"<moqui-conf><server-stats bin-length-seconds="1">
            <artifact-stats type="AT_XML_SCREEN" persist-bin="true" persist-hit="true"/>
            <artifact-stats type="AT_XML_SCREEN_TRANS" persist-bin="true" persist-hit="true"/>
            <artifact-stats type="AT_SERVICE" persist-bin="true" persist-hit="true"/>
            <artifact-stats type="AT_ENTITY" persist-bin="true"/>
        </server-stats></moqui-conf>"#;
    world_with_conf(conf)
}

fn world_with_conf(conf: &str) -> World {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("moqui-hits-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("HitScreen.xml"), SCREEN).unwrap();
    std::fs::write(dir.join("Untracked.xml"), UNTRACKED).unwrap();
    std::fs::write(dir.join("HitServices.xml"), SERVICES).unwrap();
    let pools = crate::service::ServicePools::bounded(1, 4);
    let context = crate::context::ExecutionContextFactory::test_context_with(conf, pools);
    let tape = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let facade = crate::testing::open_with_tape(std::sync::Arc::clone(&tape));
    context.factory().set_entity(facade.clone());
    {
        let _entered = context.enter();
        let failures = crate::service::lint(&[dir.join("HitServices.xml")]).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
    }
    World {
        context,
        facade,
        tape,
        dir,
    }
}

fn rows(world: &World, entity: &str, artifact_type: &str) -> Vec<crate::entity::EntityValue> {
    world
        .facade
        .find(entity)
        .unwrap()
        .eq("artifactType", Value::string(artifact_type))
        .list()
        .unwrap()
        .values()
        .to_vec()
}

fn text(value: &crate::entity::EntityValue, field: &str) -> String {
    value
        .get(field)
        .map(moqui_lib::object::to_plain_string)
        .unwrap_or_default()
}

fn number(value: &crate::entity::EntityValue, field: &str) -> f64 {
    text(value, field).parse().unwrap_or(-1.0)
}

fn render(world: &World, screen: &str, path: &str) -> String {
    crate::screen::make_render(world.context.factory())
        .root_screen(world.dir.join(screen).to_string_lossy().into_owned())
        .screen_path(path)
        .render()
        .unwrap_or_else(|err| panic!("{screen}{path}: {err}"))
        .body_text()
}

#[test]
fn shipped_settings_and_disabled_overrides_survive_merge_and_shutdown_flush() {
    use moqui_lib::node::MNode;

    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    for mode in ["default", "dev", "disabled"] {
        let conf = MNode::parse_file(runtime.join("base-component/framework/conf/MoquiDefaultConf.xml")).unwrap();
        if mode != "default" {
            super::super::conf::merge_conf(
                &conf,
                &MNode::parse_file(runtime.join("conf/MoquiDevConf.xml")).unwrap(),
            );
        }
        if mode == "disabled" {
            super::super::conf::merge_conf(
                &conf,
                &MNode::parse_text(
                    "disabled",
                    r#"<moqui-conf><server-stats>
                        <artifact-stats type="AT_XML_SCREEN" persist-hit="false" persist-bin="false"/>
                        <artifact-stats type="AT_SERVICE" persist-hit="false" persist-bin="false"/>
                        <artifact-stats type="AT_ENTITY" persist-bin="false"/>
                    </server-stats></moqui-conf>"#,
                )
                .unwrap(),
            );
        }
        let world = world_with_conf(&conf.write_string());
        let configured = world.context.factory().artifact_hits();
        assert_eq!(configured.bin_length_ms, 900_000);
        assert_eq!(configured.persist("AT_XML_SCREEN_CONTENT"), (mode != "default", true));
        assert_eq!(configured.persist("AT_LLM"), (false, true));
        assert_eq!(configured.persist("AT_ENTITY"), (false, mode == "dev"));
        let _entered = world.context.enter();
        let _authz = world.context.disable_authz();
        assert!(render(&world, "HitScreen.xml", "").contains("tracked"));
        render(&world, "Untracked.xml", "");
        crate::service::sync("find#Hits", &BTreeMap::new()).unwrap();
        // No tick and no bin expiry: shutdown must persist the still-open bins and queued hits.
        assert!(rows(&world, super::HIT_BIN, "AT_SERVICE").is_empty());
        world.context.factory().flush_artifact_hits().unwrap();
        for (kind, hit_enabled, bin_enabled) in [
            ("AT_XML_SCREEN", mode != "disabled", mode != "disabled"),
            ("AT_SERVICE", mode == "dev", mode != "disabled"),
            ("AT_ENTITY", false, mode == "dev"),
        ] {
            assert_eq!(
                !rows(&world, super::HIT, kind).is_empty(),
                hit_enabled,
                "{mode}: {kind} hits"
            );
            let bins = rows(&world, super::HIT_BIN, kind);
            assert_eq!(!bins.is_empty(), bin_enabled, "{mode}: {kind} bins");
            assert!(bins.iter().all(|bin| number(bin, "hitCount") > 0.0));
            assert!(bins
                .iter()
                .all(|bin| !text(bin, "artifactName").ends_with("Untracked.xml")));
        }
        let before = rows(&world, super::HIT_BIN, "AT_XML_SCREEN").len();
        world.context.factory().flush_artifact_hits().unwrap();
        assert_eq!(rows(&world, super::HIT_BIN, "AT_XML_SCREEN").len(), before);
    }
}

#[test]
fn create_or_update_counts_what_it_runs_and_an_unmatched_probe_apart() {
    let world = world();
    let _entered = world.context.enter();
    let _authz = world.context.disable_authz();
    let entity = "moqui.basic.EnumerationType";
    let mut value = world.facade.make_value(entity).unwrap();
    value.set("enumTypeId", Value::string("HitType"));
    value.set("description", Value::string("first"));
    // A new row: the UPDATE probe matches nothing (Rust-only `update-probe`), then create.
    value.create_or_update(&world.facade).unwrap();
    value.set("description", Value::string("second"));
    // An existing row: the probe is the update, with no refresh.
    value.create_or_update(&world.facade).unwrap();
    value.refresh(&world.facade).unwrap();
    world.facade.find(entity).unwrap().one().unwrap();
    world.facade.find(entity).unwrap().list().unwrap();
    world.context.factory().flush_artifact_hits().unwrap();

    let bins = rows(&world, "moqui.server.ArtifactHitBin", "AT_ENTITY");
    let counts: BTreeMap<_, _> = bins
        .iter()
        .filter(|bin| text(bin, "artifactName") == entity)
        .map(|bin| (text(bin, "artifactSubType"), number(bin, "hitCount")))
        .collect();
    assert_eq!(bins.iter().filter(|bin| text(bin, "artifactName") == entity).count(), 6);
    assert_eq!(
        counts,
        BTreeMap::from([
            ("create".to_string(), 1.0),
            ("update".to_string(), 1.0),
            ("update-probe".to_string(), 1.0),
            ("refresh".to_string(), 1.0),
            ("one".to_string(), 1.0),
            ("list".to_string(), 1.0),
        ])
    );
}

#[test]
fn audit_services_reuse_the_users_authorization_list_across_seed_writes() {
    let world = world_with_conf(
        r#"<moqui-conf>
            <artifact-execution-facade>
                <artifact-execution type="AT_SERVICE" authz-enabled="true"/>
            </artifact-execution-facade>
            <server-stats bin-length-seconds="600">
                <artifact-stats type="AT_SERVICE" persist-bin="true"/>
                <artifact-stats type="AT_ENTITY" persist-bin="true"/>
            </server-stats>
        </moqui-conf>"#,
    );
    let _entered = world.context.enter();
    let _authz = world.context.disable_authz();
    world.facade.set_disable_audit_log(false);
    let mut user = world.facade.make_value("moqui.security.UserAccount").unwrap();
    user.set("userId", Value::string("AUDIT_CACHE"));
    user.set("username", Value::string("first"));
    user.create(&world.facade).unwrap();
    let mut authz = world.facade.make_value("moqui.security.ArtifactAuthz").unwrap();
    authz.set("artifactAuthzId", Value::string("SEED_CACHE"));
    authz.set("userGroupId", Value::string("ALL_USERS"));
    authz.create(&world.facade).unwrap();
    user.set("username", Value::string("second"));
    user.update(&world.facade).unwrap();
    world.context.factory().flush_artifact_hits().unwrap();
    let bins = rows(&world, "moqui.server.ArtifactHitBin", "AT_ENTITY");
    let checks: Vec<_> = bins
        .iter()
        .filter(|bin| {
            text(bin, "artifactName") == "moqui.security.ArtifactAuthzCheckView"
                && text(bin, "artifactSubType") == "list"
        })
        .collect();
    assert_eq!(checks.len(), 1);
    assert_eq!(text(checks[0], "hitCount"), "1");
    let bins = rows(&world, "moqui.server.ArtifactHitBin", "AT_SERVICE");
    let audit: Vec<_> = bins
        .iter()
        .filter(|bin| text(bin, "artifactName") == "create#moqui.entity.EntityAuditLog")
        .collect();
    assert_eq!(audit.len(), 1);
    assert_eq!(text(audit[0], "artifactSubType"), "entity-implicit");
    assert_eq!(text(audit[0], "hitCount"), "2");
}

#[test]
fn fk_reads_are_counted_apart_when_a_load_makes_dummy_parents() {
    let world = world();
    let _entered = world.context.enter();
    let _authz = world.context.disable_authz();
    world.facade.set_disable_fk_create(false);
    let mut flow = world.facade.make_value("moqui.basic.StatusFlow").unwrap();
    flow.set("statusFlowId", Value::string("Default"));
    flow.create(&world.facade).unwrap();
    let mut transition = world.facade.make_value("moqui.basic.StatusFlowTransition").unwrap();
    transition.set("statusFlowId", Value::string("Default"));
    transition.set("statusId", Value::string("Missing"));
    transition.set("toStatusId", Value::string("AlsoMissing"));
    // Foreign keys are only checked with dummy foreign keys, as Java's data loader does.
    world.facade.set_dummy_fks(true);
    transition.create(&world.facade).unwrap();
    world.facade.set_dummy_fks(false);
    // A read made after the FK check is counted under its own sub type again.
    world.facade.find("moqui.basic.StatusFlow").unwrap().list().unwrap();
    world.context.factory().flush_artifact_hits().unwrap();
    let bins = rows(&world, "moqui.server.ArtifactHitBin", "AT_ENTITY");
    let status_items: Vec<String> = bins
        .iter()
        .filter(|bin| text(bin, "artifactName") == "moqui.basic.StatusItem")
        .map(|bin| text(bin, "artifactSubType"))
        .collect();
    // The check reads are `fk-check`, not `one`; the dummy row is a create.
    assert_eq!(status_items, ["create", "fk-check"]);
    assert!(bins
        .iter()
        .any(|bin| text(bin, "artifactName") == "moqui.basic.StatusFlow" && text(bin, "artifactSubType") == "list"));
}

#[test]
fn existence_reads_do_not_localize_unused_fields_or_disable_later_localization() {
    let world = world();
    let _entered = world.context.enter();
    let _authz = world.context.disable_authz();
    world.facade.set_disable_fk_create(false);
    for id in ["First", "Second"] {
        let mut status = world.facade.make_value("moqui.basic.StatusItem").unwrap();
        status.set("statusId", Value::string(id));
        status.set("description", Value::string(id));
        status.create(&world.facade).unwrap();
        status.create_or_update(&world.facade).unwrap();
    }
    let mut flow = world.facade.make_value("moqui.basic.StatusFlow").unwrap();
    flow.set("statusFlowId", Value::string("Default"));
    flow.create(&world.facade).unwrap();
    let mut transition = world.facade.make_value("moqui.basic.StatusFlowTransition").unwrap();
    transition.set("statusFlowId", Value::string("Default"));
    transition.set("statusId", Value::string("First"));
    transition.set("toStatusId", Value::string("Second"));
    transition.create(&world.facade).unwrap();
    world.context.factory().flush_artifact_hits().unwrap();
    let bins = rows(&world, "moqui.server.ArtifactHitBin", "AT_ENTITY");
    assert!(bins
        .iter()
        .all(|bin| text(bin, "artifactName") != "moqui.basic.LocalizedEntityField"));
    // The two FK existence reads of the transition ran, but Java makes no such read, so they are not hits.
    assert!(bins
        .iter()
        .all(|bin| !(text(bin, "artifactName") == "moqui.basic.StatusItem" && text(bin, "artifactSubType") == "one")));

    let mut translation = world.facade.make_value("moqui.basic.LocalizedEntityField").unwrap();
    for (field, value) in [
        ("entityName", "moqui.basic.StatusItem"),
        ("fieldName", "description"),
        ("pkValue", "First"),
        ("locale", "en"),
        ("localized", "Translated"),
    ] {
        translation.set(field, Value::string(value));
    }
    translation.create(&world.facade).unwrap();
    let found = world
        .facade
        .find("moqui.basic.StatusItem")
        .unwrap()
        .eq("statusId", Value::string("First"))
        .one()
        .unwrap()
        .unwrap();
    assert_eq!(
        found.get_localized(&world.facade, "description"),
        Some(Value::string("Translated"))
    );
    assert_eq!(found.get("description"), Some(&Value::string("First")));
}

#[test]
fn plain_map_loader_upserts_each_row_with_its_whole_key() {
    let world = world();
    let _entered = world.context.enter();
    let _authz = world.context.disable_authz();
    world.facade.set_disable_fk_create(true);
    crate::entity::load_feed::load_entity_xml_text(
        &world.facade,
        "statistics-loader",
        r#"<entity-facade-xml>
                <moqui.basic.EnumerationType enumTypeId="HitType" description="first"/>
                <moqui.basic.EnumerationType enumTypeId="HitType" description="second"/>
                <moqui.basic.StatusItem statusId="HitStatus"/>
            </entity-facade-xml>"#,
        &crate::entity::load_feed::LoadOptions::default(),
    )
    .unwrap();
    world.context.factory().flush_artifact_hits().unwrap();
    let bins = rows(&world, "moqui.server.ArtifactHitBin", "AT_ENTITY");
    let counts: BTreeMap<_, _> = bins
        .iter()
        .filter(|bin| {
            ["moqui.basic.EnumerationType", "moqui.basic.StatusItem"].contains(&text(bin, "artifactName").as_str())
        })
        .map(|bin| {
            (
                (text(bin, "artifactName"), text(bin, "artifactSubType")),
                number(bin, "hitCount"),
            )
        })
        .collect();
    // Each row is one upsert, a create's hit whether the row was there or not. The StatusItem's `statusId` is its
    // primary key, which cannot change, so it has no status change to check against a stored row.
    let expected = [
        ("moqui.basic.EnumerationType", "create", 2.0),
        ("moqui.basic.StatusItem", "create", 1.0),
    ]
    .into_iter()
    .map(|(entity, operation, count)| ((entity.to_string(), operation.to_string()), count))
    .collect();
    assert_eq!(counts, expected);
}

#[test]
fn screen_service_and_entity_hits_are_written_by_the_tick_and_shown_by_the_tools_screens() {
    let world = world();
    let _entered = world.context.enter();
    let _authz = world.context.disable_authz();
    let mut account = world.facade.make_value("moqui.security.UserAccount").unwrap();
    account.set("userId", Value::string("HitUser"));
    account.set("username", Value::string("hituser"));
    account.create(&world.facade).unwrap();
    crate::user::internal_login_user("hituser").unwrap();
    let screen = world.dir.join("HitScreen.xml").to_string_lossy().into_owned();
    let untracked = world.dir.join("Untracked.xml").to_string_lossy().into_owned();
    assert!(render(&world, "HitScreen.xml", "").contains("tracked"));
    assert!(render(&world, "HitScreen.xml", "").contains("tracked"));
    assert!(render(&world, "Untracked.xml", "").contains("untracked"));
    render(&world, "HitScreen.xml", "/doIt");
    let mut parameters = BTreeMap::new();
    parameters.insert("enumTypeId".to_string(), Value::string("HitType"));
    parameters.insert("password".to_string(), Value::string("secret"));
    crate::service::sync("find#Hits", &parameters).unwrap();
    world.facade.find("moqui.basic.Enumeration").unwrap().list().unwrap();
    world.facade.find("moqui.basic.Enumeration").unwrap().one().unwrap();

    // Nothing is written on the request path.
    assert!(rows(&world, "moqui.server.ArtifactHit", "AT_XML_SCREEN").is_empty());
    assert!(rows(&world, "moqui.server.ArtifactHitBin", "AT_SERVICE").is_empty());

    // After the bin length, one tick writes the hits and the bins that ended.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    world.context.factory().service_pools().tick();

    let screen_bins = rows(&world, "moqui.server.ArtifactHitBin", "AT_XML_SCREEN");
    // The two renders. The transition's path frame is a separate bin with no sub-type, as in Java.
    let renders: Vec<_> = screen_bins
        .iter()
        .filter(|bin| text(bin, "artifactName") == screen && text(bin, "artifactSubType").starts_with("text/html"))
        .collect();
    assert_eq!(renders.len(), 1, "{screen_bins:?}");
    assert_eq!(number(renders[0], "hitCount"), 2.0);
    assert!(number(renders[0], "minTimeMillis") <= number(renders[0], "maxTimeMillis"));
    assert!(number(renders[0], "totalTimeMillis") > 0.0);
    assert!(!text(renders[0], "binStartDateTime").is_empty());
    assert!(!text(renders[0], "serverHostName").is_empty());
    assert!(screen_bins.iter().all(|bin| text(bin, "artifactName") != untracked));

    let transitions = rows(&world, "moqui.server.ArtifactHitBin", "AT_XML_SCREEN_TRANS");
    let names: Vec<String> = transitions.iter().map(|bin| text(bin, "artifactName")).collect();
    assert!(names.contains(&format!("{screen}#doIt")), "{names:?}");
    assert!(names.contains(&format!("{screen}/doIt")), "{names:?}");

    let services = rows(&world, "moqui.server.ArtifactHitBin", "AT_SERVICE");
    assert_eq!(services.len(), 1, "{services:?}");
    assert_eq!(text(&services[0], "artifactName"), "find#Hits");
    assert_eq!(text(&services[0], "artifactSubType"), "inline");
    assert_eq!(number(&services[0], "hitCount"), 1.0);

    let entities = rows(&world, "moqui.server.ArtifactHitBin", "AT_ENTITY");
    let lists: Vec<_> = entities
        .iter()
        .filter(|bin| text(bin, "artifactName") == "moqui.basic.Enumeration" && text(bin, "artifactSubType") == "list")
        .collect();
    assert_eq!(lists.len(), 1, "{entities:?}");
    assert_eq!(number(lists[0], "hitCount"), 2.0);
    let ones: Vec<_> = entities
        .iter()
        .filter(|bin| text(bin, "artifactName") == "moqui.basic.Enumeration" && text(bin, "artifactSubType") == "one")
        .collect();
    assert_eq!(ones.len(), 1, "{entities:?}");
    assert_eq!(number(ones[0], "hitCount"), 1.0);
    // The hit tables themselves are not counted.
    assert!(entities
        .iter()
        .all(|bin| !text(bin, "artifactName").starts_with("moqui.server.ArtifactHit")));

    let screen_hits: Vec<_> = rows(&world, "moqui.server.ArtifactHit", "AT_XML_SCREEN")
        .into_iter()
        .filter(|hit| text(hit, "artifactName") == screen && text(hit, "artifactSubType").starts_with("text/html"))
        .collect();
    assert_eq!(screen_hits.len(), 2);
    assert_eq!(text(&screen_hits[0], "wasError"), "N");
    assert_eq!(text(&screen_hits[0], "isSlowHit"), "N");
    assert_eq!(text(&screen_hits[0], "userId"), "HitUser");
    assert!(number(&screen_hits[0], "runningTimeMillis") >= 0.0);
    assert!(rows(&world, "moqui.server.ArtifactHit", "AT_XML_SCREEN")
        .iter()
        .all(|hit| text(hit, "artifactName") != untracked));
    let service_hits = rows(&world, "moqui.server.ArtifactHit", "AT_SERVICE");
    assert_eq!(service_hits.len(), 1);
    assert_eq!(text(&service_hits[0], "parameterString"), "enumTypeId=HitType");
    assert!(rows(&world, "moqui.server.ArtifactHit", "AT_ENTITY").is_empty());

    // The tools screens read the tables.
    let tools = format!(
        "{}/../../runtime/base-component/tools/screen/System",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut find = BTreeMap::new();
    find.insert("artifactName".to_string(), Value::string("find#Hits"));
    let bins = crate::screen::make_render(world.context.factory())
        .root_screen(format!("{tools}/ArtifactHitBins.xml"))
        .parameters(find.clone())
        .render()
        .unwrap()
        .body_text();
    assert!(!bins.contains("Error rendering"), "{bins}");
    assert!(bins.contains("find#Hits") && bins.contains("AT_SERVICE"), "{bins}");
    let summary = crate::screen::make_render(world.context.factory())
        .root_screen(format!("{tools}/ArtifactHitSummary.xml"))
        .render()
        .unwrap()
        .body_text();
    assert!(!summary.contains("Error rendering"), "{summary}");
    assert!(summary.contains("ArtifactHitSummaryList"), "{summary}");
    // The in-memory database has no aggregates, so the summary's query is checked as it is sent: the
    // displayed aliases are grouped.
    let _ = crate::screen::make_render(world.context.factory())
        .root_screen(format!("{tools}/ArtifactHitSummary.xml"))
        .parameters(find)
        .render();
    let sql = world
        .tape
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|sql| sql.contains("SUM("))
        .cloned()
        .unwrap_or_default();
    assert!(sql.contains("GROUP BY") && sql.contains("\"artifactName\""), "{sql}");
    crate::user::session::reset_session();
}
