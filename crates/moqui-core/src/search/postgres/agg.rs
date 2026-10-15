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

//! Aggregation trees to SQL, one statement per bucket level.
//!
//! A level groups by the keys of every bucket aggregation above it plus its own. Metrics and
//! filter counts that sit in the level, with no bucket aggregation between, are columns of that
//! statement. A `Filter` adds `FILTER (WHERE ..)` to the columns below it, and becomes a `WHERE`
//! condition for any bucket level below it. A sum of a child-array number is a lateral per-row
//! sum, so the total sees every element, as Elastic's `include_in_root` does.
//!
//! Terms ordering, terms size, and date histogram gap buckets are done in Rust on the grouped rows.

use std::collections::HashMap;

use indexmap::IndexMap;
use jiff::tz::TimeZone;
use jiff::Timestamp;
use moqui_lib::error::Error;
use moqui_lib::object::{compare_obj, to_plain_string, Value};

use super::compile::{literal, Compiler, Located, Sql, TextSetup};
use crate::driver::DbRow;
use crate::search::agg::{
    bucket_start, histogram_zone, key_display, next_bucket, zone_name, Agg, DateHistogram, Terms, TermsOrder,
};
use crate::search::agg_empty_results;
use crate::search::mapping::{DocumentMapping, FieldKind};
use crate::search::query::Query;

/// Runs one statement.
pub type Runner<'r> = dyn FnMut(&Sql) -> Result<Vec<DbRow>, Error> + 'r;

/// The most groups one level may return before Rust orders and trims them.
const MAX_GROUPS: usize = 65_536;

#[derive(Clone)]
struct KeyDef {
    select: String,
    lateral: Option<String>,
}

enum Item {
    /// `count(*)` under the filters on the path, stored at `path` + `doc_count`.
    DocCount { path: Vec<String>, conds: Vec<Sql> },
    Sum {
        path: Vec<String>,
        conds: Vec<Sql>,
        field: String,
    },
    CountDistinct {
        path: Vec<String>,
        conds: Vec<Sql>,
        field: String,
    },
}

struct ChildLevel<'t> {
    path: Vec<String>,
    conds: Vec<Sql>,
    agg: &'t Agg,
}

type Row = (Vec<Value>, IndexMap<String, Value>);
/// A child level's rows by the parent key tuple: the own key and the bucket content.
type Grouped = HashMap<String, Vec<(Value, IndexMap<String, Value>)>>;

/// Run `aggs` over the documents matching `filter`.
///
/// # Errors
///
/// Returns an error when a field cannot be aggregated as asked, or a statement fails.
pub fn aggregate(
    table: &str,
    mapping: &DocumentMapping,
    text: &TextSetup,
    filter: &Query,
    aggs: &IndexMap<String, Agg>,
    run: &mut Runner<'_>,
) -> Result<IndexMap<String, Value>, Error> {
    let compiler = Compiler::with_text(mapping, text);
    let base = compiler.where_sql(filter)?;
    let rows = run_level(table, mapping, text, &base, &[], &[], aggs, run)?;
    let mut root = rows
        .into_iter()
        .next()
        .map(|(_, content)| content)
        .unwrap_or_else(|| agg_empty_results(aggs));
    root.shift_remove("doc_count");
    Ok(root)
}

#[allow(clippy::too_many_arguments)]
fn run_level(
    table: &str,
    mapping: &DocumentMapping,
    text: &TextSetup,
    base: &Sql,
    keys: &[KeyDef],
    conds: &[Sql],
    aggs: &IndexMap<String, Agg>,
    run: &mut Runner<'_>,
) -> Result<Vec<Row>, Error> {
    let compiler = Compiler::with_text(mapping, text);
    let mut items = Vec::new();
    let mut children = Vec::new();
    collect(&compiler, aggs, &[], &[], &mut items, &mut children)?;

    let mut sql = Sql::new("SELECT ");
    for (index, key) in keys.iter().enumerate() {
        sql.push(&format!("{} AS k{index}, ", key.select));
    }
    sql.push("count(*) AS dc");
    let mut laterals = Vec::new();
    for (index, item) in items.iter().enumerate() {
        sql.push(", ");
        match item {
            Item::DocCount { conds, .. } => {
                sql.push("count(*)");
                filter_clause(&mut sql, conds);
            }
            Item::Sum { conds, field, .. } => {
                let value = match compiler.locate_root(field) {
                    Located::Missing => "NULL::numeric".to_string(),
                    Located::Field {
                        arrays,
                        expr,
                        field: found,
                    } => {
                        if !matches!(found.kind, FieldKind::Long | FieldKind::Double) {
                            return Err(Error::new(format!("sum needs a number field, {field} is not")));
                        }
                        if arrays.is_empty() {
                            expr
                        } else {
                            let alias = format!("s{index}");
                            laterals.push(format!(
                                "LEFT JOIN LATERAL (SELECT sum({expr}) AS v FROM {}) AS {alias} ON TRUE",
                                from_list(&arrays)
                            ));
                            format!("{alias}.v")
                        }
                    }
                };
                sql.push(&format!("COALESCE(sum({value})"));
                filter_clause(&mut sql, conds);
                sql.push(", 0)");
            }
            Item::CountDistinct { conds, field, .. } => {
                let value = match compiler.locate_root(field) {
                    Located::Missing => "NULL".to_string(),
                    Located::Field { arrays, expr, .. } => {
                        if !arrays.is_empty() {
                            return Err(Error::new(format!(
                                "count_distinct of the child field {field} is not supported"
                            )));
                        }
                        expr
                    }
                };
                sql.push(&format!("count(DISTINCT {value})"));
                filter_clause(&mut sql, conds);
            }
        }
        sql.push(&format!(" AS c{index}"));
    }
    sql.push(&format!(" FROM {table}"));
    for key in keys {
        if let Some(lateral) = &key.lateral {
            sql.push(" ");
            sql.push(lateral);
        }
    }
    for lateral in &laterals {
        sql.push(" ");
        sql.push(lateral);
    }
    sql.push(" WHERE ");
    sql.append(base.clone());
    for cond in conds {
        sql.push(" AND ");
        sql.append(cond.clone());
    }
    for key in keys {
        sql.push(&format!(" AND {} IS NOT NULL", key.select));
    }
    if !keys.is_empty() {
        let ordinals: Vec<String> = (1..=keys.len()).map(|index| index.to_string()).collect();
        sql.push(&format!(" GROUP BY {}", ordinals.join(", ")));
        // One more than the cap is enough to tell it was exceeded, without reading every group.
        sql.push(&format!(" LIMIT {}", MAX_GROUPS + 1));
    }
    let rows = run(&sql)?;
    if rows.len() > MAX_GROUPS {
        return Err(Error::new(format!(
            "an aggregation level returned more than {MAX_GROUPS} groups"
        )));
    }

    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    for row in &rows {
        let tuple: Vec<Value> = (0..keys.len())
            .map(|index| row.get(&format!("k{index}")).cloned().unwrap_or(Value::Null))
            .collect();
        let mut content = IndexMap::new();
        content.insert("doc_count".to_string(), count_value(row.get("dc")));
        for (name, value) in agg_empty_results(aggs) {
            content.insert(name, value);
        }
        for (index, item) in items.iter().enumerate() {
            let cell = row.get(&format!("c{index}")).cloned().unwrap_or(Value::Null);
            match item {
                Item::DocCount { path, .. } => {
                    let mut full = path.clone();
                    full.push("doc_count".to_string());
                    set_path(&mut content, &full, count_value(Some(&cell)));
                }
                Item::Sum { path, .. } => set_path(&mut content, path, sum_value(cell)),
                Item::CountDistinct { path, .. } => set_path(&mut content, path, count_value(Some(&cell))),
            }
        }
        out.push((tuple, content));
    }

    for child in children {
        let (key, sub_aggs) = match child.agg {
            Agg::DateHistogram(histogram) => (histogram_key(&compiler, histogram, keys.len())?, &histogram.aggs),
            Agg::Terms(terms) => (terms_key(&compiler, terms, keys.len())?, &terms.aggs),
            _ => continue,
        };
        let mut child_keys: Vec<KeyDef> = keys.to_vec();
        child_keys.push(key);
        let mut child_conds: Vec<Sql> = conds.to_vec();
        child_conds.extend(child.conds.iter().cloned());
        let child_rows = run_level(table, mapping, text, base, &child_keys, &child_conds, sub_aggs, run)?;
        let mut grouped: Grouped = HashMap::new();
        for (tuple, content) in child_rows {
            let (own, parent) = tuple
                .split_last()
                .map(|(own, parent)| (own.clone(), parent.to_vec()))
                .unwrap_or((Value::Null, Vec::new()));
            grouped.entry(tuple_key(&parent)).or_default().push((own, content));
        }
        for (parent_tuple, parent_content) in &mut out {
            let rows = grouped.remove(&tuple_key(parent_tuple)).unwrap_or_default();
            let buckets = match child.agg {
                Agg::DateHistogram(histogram) => histogram_buckets(histogram, rows)?,
                Agg::Terms(terms) => terms_buckets(terms, rows),
                _ => Vec::new(),
            };
            let mut holder = IndexMap::new();
            holder.insert("buckets".to_string(), Value::List(buckets));
            set_path(parent_content, &child.path, Value::Map(holder));
        }
    }
    Ok(out)
}

fn collect<'t>(
    compiler: &Compiler<'_>,
    aggs: &'t IndexMap<String, Agg>,
    path: &[String],
    conds: &[Sql],
    items: &mut Vec<Item>,
    children: &mut Vec<ChildLevel<'t>>,
) -> Result<(), Error> {
    for (name, agg) in aggs {
        let mut here = path.to_vec();
        here.push(name.clone());
        match agg {
            Agg::Sum { field } => items.push(Item::Sum {
                path: here,
                conds: conds.to_vec(),
                field: field.clone(),
            }),
            Agg::CountDistinct { field } => items.push(Item::CountDistinct {
                path: here,
                conds: conds.to_vec(),
                field: field.clone(),
            }),
            Agg::Filter { query, aggs } => {
                let mut inner = conds.to_vec();
                inner.push(compiler.where_sql(query)?);
                items.push(Item::DocCount {
                    path: here.clone(),
                    conds: inner.clone(),
                });
                collect(compiler, aggs, &here, &inner, items, children)?;
            }
            Agg::DateHistogram(_) | Agg::Terms(_) => children.push(ChildLevel {
                path: here,
                conds: conds.to_vec(),
                agg,
            }),
        }
    }
    Ok(())
}

fn filter_clause(sql: &mut Sql, conds: &[Sql]) {
    if conds.is_empty() {
        return;
    }
    sql.push(" FILTER (WHERE ");
    for (index, cond) in conds.iter().enumerate() {
        if index > 0 {
            sql.push(" AND ");
        }
        sql.append(cond.clone());
    }
    sql.push(")");
}

fn from_list(arrays: &[(String, String)]) -> String {
    arrays
        .iter()
        .map(|(var, source)| format!("{source} AS {var}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn safe_zone(zone: &TimeZone) -> Result<String, Error> {
    let name = zone_name(zone);
    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '+' | '-' | ':'))
    {
        Ok(name)
    } else {
        Err(Error::new(format!("time zone {name} cannot be used in a histogram")))
    }
}

fn histogram_key(compiler: &Compiler<'_>, histogram: &DateHistogram, depth: usize) -> Result<KeyDef, Error> {
    let zone = histogram_zone(histogram.time_zone.as_deref())?;
    let zone_text = literal(&safe_zone(&zone)?);
    let offset = format!("interval '{} seconds'", histogram.offset.as_secs());
    let (value, lateral) = date_value(compiler, &histogram.field, depth)?;
    let select = format!(
        "((date_trunc('{}', ({value} AT TIME ZONE {zone_text}) - {offset}) + {offset}) AT TIME ZONE {zone_text})",
        histogram.unit.trunc_field()
    );
    Ok(KeyDef { select, lateral })
}

fn date_value(compiler: &Compiler<'_>, field: &str, depth: usize) -> Result<(String, Option<String>), Error> {
    match compiler.locate_root(field) {
        Located::Missing => Ok(("NULL::timestamptz".to_string(), None)),
        Located::Field {
            arrays,
            expr,
            field: found,
        } => {
            if found.kind != FieldKind::Date {
                return Err(Error::new(format!(
                    "a date histogram needs a date field, {field} is not"
                )));
            }
            if arrays.is_empty() {
                Ok((expr, None))
            } else {
                let alias = format!("k{depth}d");
                Ok((
                    format!("{alias}.v"),
                    Some(format!(
                        "CROSS JOIN LATERAL (SELECT DISTINCT {expr} AS v FROM {}) AS {alias}",
                        from_list(&arrays)
                    )),
                ))
            }
        }
    }
}

fn terms_key(compiler: &Compiler<'_>, terms: &Terms, depth: usize) -> Result<KeyDef, Error> {
    match compiler.locate_root(&terms.field) {
        Located::Missing => Ok(KeyDef {
            select: "NULL::text".to_string(),
            lateral: None,
        }),
        Located::Field { arrays, expr, field } => {
            if field.kind == FieldKind::Text && !field.sortable {
                return Err(Error::new(format!(
                    "terms on the text field {} needs sortable=Y",
                    terms.field
                )));
            }
            if arrays.is_empty() {
                Ok(KeyDef {
                    select: expr,
                    lateral: None,
                })
            } else {
                let alias = format!("k{depth}t");
                Ok(KeyDef {
                    select: format!("{alias}.v"),
                    lateral: Some(format!(
                        "CROSS JOIN LATERAL (SELECT DISTINCT {expr} AS v FROM {}) AS {alias}",
                        from_list(&arrays)
                    )),
                })
            }
        }
    }
}

fn histogram_buckets(
    histogram: &DateHistogram,
    rows: Vec<(Value, IndexMap<String, Value>)>,
) -> Result<Vec<Value>, Error> {
    let zone = histogram_zone(histogram.time_zone.as_deref())?;
    let mut by_key: Vec<(Timestamp, IndexMap<String, Value>)> = Vec::with_capacity(rows.len());
    for (key, content) in rows {
        let Some(at) = timestamp_of(&key) else {
            continue;
        };
        by_key.push((at, content));
    }
    by_key.sort_by_key(|(at, _)| *at);
    if histogram.min_doc_count == 0 && by_key.len() > 1 {
        let first = by_key[0].0;
        let last = by_key[by_key.len() - 1].0;
        let mut filled = Vec::new();
        let mut existing = by_key.into_iter().peekable();
        // Normalize, in case the database bucket and the Rust bucket disagree at a zone edge.
        let mut at = bucket_start(first, histogram.unit, &zone, histogram.offset)?;
        let mut guard = 0usize;
        while at <= last {
            guard += 1;
            if guard > MAX_GROUPS {
                return Err(Error::new("a date histogram would have too many buckets"));
            }
            match existing.peek() {
                Some((key, _)) if *key == at => {
                    if let Some(entry) = existing.next() {
                        filled.push(entry);
                    }
                }
                Some((key, _)) if *key < at => {
                    if let Some(entry) = existing.next() {
                        filled.push(entry);
                    }
                    continue;
                }
                _ => {
                    let mut empty = IndexMap::new();
                    empty.insert("doc_count".to_string(), Value::I64(0));
                    for (name, value) in agg_empty_results(&histogram.aggs) {
                        empty.insert(name, value);
                    }
                    filled.push((at, empty));
                }
            }
            at = next_bucket(at, histogram.unit, &zone, histogram.offset)?;
        }
        filled.extend(existing);
        by_key = filled;
    }
    if histogram.min_doc_count > 1 {
        by_key.retain(|(_, content)| doc_count(content) >= histogram.min_doc_count);
    }
    if histogram.descending {
        by_key.reverse();
    }
    let mut out = Vec::with_capacity(by_key.len());
    for (at, content) in by_key {
        let mut bucket = IndexMap::new();
        bucket.insert("key".to_string(), Value::Timestamp(at));
        bucket.insert(
            "key_display".to_string(),
            Value::String(key_display(at, &zone, histogram.format.as_deref())?),
        );
        for (name, value) in content {
            bucket.insert(name, value);
        }
        out.push(Value::Map(bucket));
    }
    Ok(out)
}

fn terms_buckets(terms: &Terms, rows: Vec<(Value, IndexMap<String, Value>)>) -> Vec<Value> {
    let mut rows = rows;
    let key_order =
        |a: &Value, b: &Value| compare_obj(a, b).unwrap_or_else(|_| to_plain_string(a).cmp(&to_plain_string(b)));
    rows.sort_by(|(key_a, a), (key_b, b)| {
        let primary = match &terms.order {
            TermsOrder::Count { descending } => {
                let order = doc_count(a).cmp(&doc_count(b));
                if *descending {
                    order.reverse()
                } else {
                    order
                }
            }
            TermsOrder::Key { descending } => {
                let order = key_order(key_a, key_b);
                if *descending {
                    order.reverse()
                } else {
                    order
                }
            }
            TermsOrder::Metric { name, descending } => {
                let left = a.get(name).cloned().unwrap_or(Value::Null);
                let right = b.get(name).cloned().unwrap_or(Value::Null);
                let order = compare_obj(&left, &right).unwrap_or(std::cmp::Ordering::Equal);
                if *descending {
                    order.reverse()
                } else {
                    order
                }
            }
        };
        primary.then_with(|| key_order(key_a, key_b))
    });
    rows.truncate(terms.size);
    rows.into_iter()
        .map(|(key, content)| {
            let mut bucket = IndexMap::new();
            bucket.insert("key".to_string(), key);
            for (name, value) in content {
                bucket.insert(name, value);
            }
            Value::Map(bucket)
        })
        .collect()
}

fn doc_count(content: &IndexMap<String, Value>) -> u64 {
    match content.get("doc_count") {
        Some(Value::I64(count)) => u64::try_from(*count).unwrap_or(0),
        Some(Value::I32(count)) => u64::try_from(*count).unwrap_or(0),
        _ => 0,
    }
}

fn count_value(value: Option<&Value>) -> Value {
    match value {
        Some(Value::I64(count)) => Value::I64(*count),
        Some(Value::I32(count)) => Value::I64(i64::from(*count)),
        Some(other) if !other.is_null() => to_plain_string(other).parse::<i64>().map_or(Value::I64(0), Value::I64),
        _ => Value::I64(0),
    }
}

fn sum_value(value: Value) -> Value {
    match value {
        Value::Decimal(_) => value,
        Value::Null => Value::Decimal(Default::default()),
        other => match moqui_lib::object::basic_convert(&other, "BigDecimal").value {
            decimal @ Value::Decimal(_) => decimal,
            _ => Value::Decimal(Default::default()),
        },
    }
}

fn timestamp_of(value: &Value) -> Option<Timestamp> {
    match value {
        Value::Timestamp(at) => Some(*at),
        other => super::compile::parse_date_bound(other),
    }
}

fn tuple_key(values: &[Value]) -> String {
    values.iter().map(to_plain_string).collect::<Vec<_>>().join("\u{1}")
}

fn set_path(map: &mut IndexMap<String, Value>, path: &[String], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut current = map;
    for name in parents {
        let entry = current
            .entry(name.clone())
            .or_insert_with(|| Value::Map(IndexMap::new()));
        if !matches!(entry, Value::Map(_)) {
            *entry = Value::Map(IndexMap::new());
        }
        let Value::Map(next) = entry else {
            return;
        };
        current = next;
    }
    current.insert(last.clone(), value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::mapping::{FieldMapping, ObjectMapping};

    fn sales() -> DocumentMapping {
        let field = |kind| FieldMapping { kind, sortable: false };
        let mut root = ObjectMapping::default();
        for (name, kind) in [
            ("orderId", FieldKind::Keyword),
            ("productId", FieldKind::Keyword),
            ("itemTypeEnumId", FieldKind::Keyword),
            ("placedDate", FieldKind::Date),
            ("itemSaleTotal", FieldKind::Double),
        ] {
            root.fields.insert(name.into(), field(kind));
        }
        let mut issuances = ObjectMapping::default();
        issuances
            .fields
            .insert("issuedSaleTotal".into(), field(FieldKind::Double));
        root.children.insert("issuances".into(), issuances);
        DocumentMapping {
            document_id: "MantleSalesOrderItem".into(),
            index_name: Some("mantle_sales".into()),
            physical_name: "mantle_sales_order_item".into(),
            root,
            manual_mapping_service: None,
            display: Default::default(),
        }
    }

    fn tree(text: &str) -> IndexMap<String, Agg> {
        Agg::map_from_value(&moqui_lib::json::from_json(&serde_json::from_str(text).unwrap())).unwrap()
    }

    #[test]
    fn levels_group_filter_and_lateral_sum() {
        let aggs = tree(
            r#"{"DateOuter":{"date_histogram":{"field":"placedDate","unit":"week","time_zone":"UTC","offset":"-1d"},
                "aggs":{"orderCount":{"count_distinct":{"field":"orderId"}},
                        "ItemProduct":{"filter":{"term":{"itemTypeEnumId":"ItemProduct"}},
                            "aggs":{"productIssuedTotal":{"sum":{"field":"issuances.issuedSaleTotal"}}}}}}}"#,
        );
        let mut statements = Vec::new();
        let mut run = |sql: &Sql| -> Result<Vec<DbRow>, Error> {
            statements.push(sql.clone());
            Ok(Vec::new())
        };
        let out = aggregate(
            "\"search\".\"t\"",
            &sales(),
            &TextSetup::default(),
            &Query::MatchAll,
            &aggs,
            &mut run,
        )
        .unwrap();
        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0].text,
            "SELECT count(*) AS dc FROM \"search\".\"t\" WHERE TRUE"
        );
        let level = &statements[1].text;
        // JSON object keys come back sorted, so ItemProduct is collected before orderCount.
        assert!(level.starts_with(
            "SELECT ((date_trunc('week', (\"f_placedDate\" AT TIME ZONE 'UTC') - interval '-86400 seconds') + interval '-86400 seconds') AT TIME ZONE 'UTC') AS k0, count(*) AS dc, "
        ), "{level}");
        assert!(
            level.contains("count(*) FILTER (WHERE COALESCE(\"f_itemTypeEnumId\" = ?, FALSE)) AS c0"),
            "{level}"
        );
        assert!(
            level.contains("COALESCE(sum(s1.v) FILTER (WHERE COALESCE(\"f_itemTypeEnumId\" = ?, FALSE)), 0) AS c1"),
            "{level}"
        );
        assert!(level.contains("count(DISTINCT \"f_orderId\") AS c2"), "{level}");
        assert!(level.contains("LEFT JOIN LATERAL (SELECT sum((e"), "{level}");
        assert!(level.ends_with("GROUP BY 1 LIMIT 65537"), "{level}");
        let Value::Map(outer) = &out["DateOuter"] else { panic!() };
        assert_eq!(outer["buckets"], Value::List(Vec::new()));
    }

    #[test]
    fn a_level_past_the_group_cap_is_an_error() {
        let aggs = tree(r#"{"P":{"terms":{"field":"productId","size":10}}}"#);
        let mut run = |sql: &Sql| -> Result<Vec<DbRow>, Error> {
            if !sql.text.contains("GROUP BY") {
                return Ok(vec![DbRow {
                    columns: vec![("dc".into(), Value::I64(1))],
                }]);
            }
            // What the database returns for the LIMIT when there are more groups than that.
            let limit: usize = sql.text.rsplit(' ').next().unwrap().parse().unwrap();
            assert_eq!(limit, MAX_GROUPS + 1);
            let row = DbRow {
                columns: vec![("k0".into(), Value::string("p")), ("dc".into(), Value::I64(1))],
            };
            Ok(vec![row; limit])
        };
        let err = aggregate(
            "\"search\".\"t\"",
            &sales(),
            &TextSetup::default(),
            &Query::MatchAll,
            &aggs,
            &mut run,
        )
        .unwrap_err();
        assert!(err.message().contains("more than 65536 groups"), "{err}");
    }

    #[test]
    fn gaps_fill_between_first_and_last() {
        let aggs = tree(
            r#"{"D":{"date_histogram":{"field":"placedDate","unit":"day","time_zone":"UTC","order":"desc","format":"dd MMM yyyy"},"aggs":{"t":{"sum":{"field":"itemSaleTotal"}}}}}"#,
        );
        let Agg::DateHistogram(histogram) = &aggs["D"] else {
            panic!()
        };
        let day = |text: &str| Value::Timestamp(text.parse().unwrap());
        let mut first = IndexMap::new();
        first.insert("doc_count".to_string(), Value::I64(2));
        let mut third = IndexMap::new();
        third.insert("doc_count".to_string(), Value::I64(1));
        let buckets = histogram_buckets(
            histogram,
            vec![
                (day("2026-01-03T00:00:00Z"), third),
                (day("2026-01-01T00:00:00Z"), first),
            ],
        )
        .unwrap();
        let shown: Vec<(String, i64)> = buckets
            .iter()
            .map(|bucket| {
                let Value::Map(bucket) = bucket else { panic!() };
                let Value::I64(count) = bucket["doc_count"] else {
                    panic!()
                };
                (to_plain_string(&bucket["key_display"]), count)
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                ("03 Jan 2026".into(), 1),
                ("02 Jan 2026".into(), 0),
                ("01 Jan 2026".into(), 2)
            ]
        );
        let Value::Map(gap) = &buckets[1] else { panic!() };
        assert!(matches!(gap["t"], Value::Decimal(_)));
    }

    #[test]
    fn terms_order_by_metric_then_key_and_trim() {
        let terms = Terms {
            field: "productId".into(),
            size: 2,
            order: TermsOrder::Metric {
                name: "totalSales".into(),
                descending: true,
            },
            aggs: IndexMap::new(),
        };
        let row = |key: &str, total: i64| {
            let mut content = IndexMap::new();
            content.insert("doc_count".to_string(), Value::I64(1));
            content.insert("totalSales".to_string(), Value::I64(total));
            (Value::string(key), content)
        };
        let buckets = terms_buckets(&terms, vec![row("B", 5), row("A", 5), row("C", 9)]);
        let keys: Vec<String> = buckets
            .iter()
            .map(|bucket| {
                let Value::Map(bucket) = bucket else { panic!() };
                to_plain_string(&bucket["key"])
            })
            .collect();
        assert_eq!(keys, vec!["C", "A"]);
    }
}
