/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
//! Reporting twins consume the same authorized columnar frames as the browser and exports.
use super::{Act, Component, ComponentState, Line, Span, fit, prop, source_of, text_of, waiting};
use crate::{
    keys::{Key, KeyPress},
    theme::Role,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub struct ReportPanel;
pub struct ReportDashboard;
pub struct TraceWaterfall;
pub struct LogTail;

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn integer(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn formatted(value: &Value, field: &Value) -> String {
    if value.is_null() {
        return field["noValue"].as_str().unwrap_or("—").into();
    }
    for mapping in field["mappings"].as_array().into_iter().flatten() {
        let exact = mapping
            .get("value")
            .is_some_and(|mapped| mapped == value || text_of(Some(mapped)) == text_of(Some(value)));
        let range = number(value)
            .zip(number(&mapping["from"]))
            .zip(number(&mapping["to"]))
            .is_some_and(|((value, from), to)| value >= from && value <= to);
        if exact || range {
            if let Some(text) = mapping["text"].as_str() {
                return text.into();
            }
        }
    }
    if field["type"] == "time" {
        return value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .and_then(|millis| jiff::Timestamp::from_millisecond(millis).ok())
            .map_or_else(|| text_of(Some(value)), |time| time.to_string());
    }
    if field["type"]
        .as_str()
        .is_some_and(|kind| kind != "number" && kind != "integer")
    {
        return text_of(Some(value));
    }
    if let Some(mut value) = number(value) {
        let suffix = match field["unit"].as_str().unwrap_or("") {
            "ns" | "duration" => {
                value /= 1_000_000.0;
                " ms"
            }
            "ms" => " ms",
            "s" => " s",
            "percent" | "%" => "%",
            "bytes" => {
                let units = [" B", " KiB", " MiB", " GiB", " TiB"];
                let mut at = 0;
                while value.abs() >= 1024.0 && at < units.len() - 1 {
                    value /= 1024.0;
                    at += 1;
                }
                units[at]
            }
            _ => "",
        };
        let decimals = field["decimals"]
            .as_u64()
            .unwrap_or(if value.fract() == 0.0 { 0 } else { 2 })
            .min(12) as usize;
        return format!("{value:.decimals$}{suffix}");
    }
    text_of(Some(value))
}

fn configured_fields(frame: &Value, panel: &Value) -> Vec<Value> {
    frame["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|field| {
            let mut configured = panel["fieldConfig"]["defaults"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            if let Some(original) = field.as_object() {
                configured.extend(original.clone());
            }
            let mut field = Value::Object(configured);
            for item in panel["fieldConfig"]["overrides"].as_array().into_iter().flatten() {
                let original = field.clone();
                let matcher = &item["match"];
                let matches = matcher["name"] == original["name"] && matcher["name"].is_string()
                    || matcher["type"] == original["type"] && matcher["type"].is_string()
                    || matcher["regex"]
                        .as_str()
                        .and_then(|pattern| regex::Regex::new(pattern).ok())
                        .is_some_and(|pattern| pattern.is_match(original["name"].as_str().unwrap_or("")));
                if matches {
                    if let (Some(target), Some(config)) = (field.as_object_mut(), item["config"].as_object()) {
                        target.extend(config.clone());
                    }
                }
            }
            (field["hidden"] != true).then_some(field)
        })
        .collect()
}

fn sparkline(values: &[Value], width: usize) -> String {
    let values: Vec<Option<f64>> = values.iter().map(number).collect();
    let min = values.iter().flatten().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().flatten().copied().fold(f64::NEG_INFINITY, f64::max);
    let bars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if values.is_empty() {
        return String::new();
    }
    (0..width.min(values.len()))
        .map(|at| {
            let index = at * values.len() / width.min(values.len());
            match values[index] {
                None => ' ',
                Some(value) => {
                    let scaled = if max > min {
                        (value - min) / (max - min) * 7.0
                    } else {
                        3.0
                    };
                    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let index = scaled.round() as usize;
                    bars[index.min(7)]
                }
            }
        })
        .collect()
}

fn is_metadata(field: &Value) -> bool {
    matches!(field["name"].as_str(), Some("partial" | "__subtotal" | "__level"))
}

fn panel_lines(frame: &Value, panel: &Value, width: u16) -> Vec<Line> {
    let mut lines = Vec::new();
    let title = panel["title"].as_str().unwrap_or("Report");
    lines.push(vec![Span::new(title, Role::Text).bold()]);
    if panel["visualization"] == "text" || panel["type"] == "text" {
        lines.extend(
            panel["text"]
                .as_str()
                .unwrap_or("")
                .lines()
                .map(|line| vec![Span::new(line, Role::Text)]),
        );
        return lines;
    }
    let fields = configured_fields(frame, panel);
    let count = fields
        .iter()
        .map(|field| field["values"].as_array().map_or(0, Vec::len))
        .max()
        .unwrap_or(0);
    if count == 0 {
        lines.push(vec![Span::new("No data in this time range", Role::Muted)]);
        return lines;
    }
    for annotation in panel["annotations"].as_array().into_iter().flatten().take(20) {
        lines.push(vec![Span::new(
            format!(
                "↳ {} {}",
                text_of(annotation.get("time")),
                text_of(annotation.get("text"))
            ),
            Role::Muted,
        )]);
    }
    let visual = panel["visualization"].as_str().unwrap_or("table");
    if matches!(visual, "stat" | "gauge" | "barGauge") {
        if let Some(field) = fields
            .iter()
            .find(|field| field["type"] == "number" && !is_metadata(field))
        {
            let values = field["values"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            lines.push(vec![
                Span::new(formatted(values.last().unwrap_or(&Value::Null), field), Role::Primary).bold(),
            ]);
            lines.push(vec![Span::new(
                sparkline(values, usize::from(width).min(60)),
                Role::Primary,
            )]);
        }
        return lines;
    }
    if matches!(visual, "timeSeries" | "timeseries" | "line" | "area") {
        for field in fields
            .iter()
            .filter(|field| field["type"] == "number" && !is_metadata(field))
        {
            let values = field["values"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            lines.push(vec![Span::new(
                format!(
                    "{} {}",
                    field["name"].as_str().unwrap_or(""),
                    sparkline(values, usize::from(width).saturating_sub(20))
                ),
                Role::Primary,
            )]);
        }
    }
    let cols = fields.len().min(8);
    if cols == 0 {
        return lines;
    }
    let col_width = (usize::from(width).saturating_sub(cols * 2) / cols).clamp(4, 36);
    lines.push(vec![
        Span::new(
            fields
                .iter()
                .take(cols)
                .map(|field| fit(field["name"].as_str().unwrap_or(""), col_width))
                .collect::<Vec<_>>()
                .join("  "),
            Role::Text,
        )
        .bold(),
    ]);
    for at in 0..count.min(100) {
        let cells: Vec<String> = fields
            .iter()
            .take(cols)
            .map(|field| fit(&formatted(&field["values"][at], field), col_width))
            .collect();
        lines.push(vec![Span::new(cells.join("  "), Role::Text)]);
    }
    if count > 100 {
        lines.push(vec![Span::new(
            format!("Showing 100 of {count} rows; use export for all rows"),
            Role::Muted,
        )]);
    }
    lines
}

impl Component for ReportPanel {
    fn name(&self) -> &'static str {
        "report-panel"
    }
    fn source(&self, props: &Value, state: &ComponentState) -> Option<(String, BTreeMap<String, String>)> {
        source_of(props, state)
    }
    fn lines(&self, props: &Value, state: &ComponentState, width: u16, _wide: bool) -> Vec<Line> {
        if let Some(line) = waiting(state) {
            return vec![line];
        }
        let data = state.data.as_ref().unwrap_or(props);
        let frame = data.get("frame").or_else(|| data["frames"].get(0)).unwrap_or(data);
        let mut lines = panel_lines(frame, &props["panel"], width);
        if state.closed.contains("__report_inspector") {
            lines.extend(
                serde_json::to_string_pretty(data)
                    .unwrap_or_default()
                    .lines()
                    .take(300)
                    .map(|line| vec![Span::new(line, Role::Muted)]),
            );
        }
        lines
    }
    fn key(&self, props: &Value, state: &mut ComponentState, press: &KeyPress, _wide: bool) -> Option<Vec<Act>> {
        if press.key == Key::Char('i') && press.is_plain() {
            if !state.closed.remove("__report_inspector") {
                state.closed.insert("__report_inspector".into());
            }
            return Some(Vec::new());
        }
        (press.key == Key::Char('r') && press.is_plain()).then(|| {
            vec![if source_of(props, state).is_some() {
                Act::Reload
            } else {
                Act::ReloadScreen
            }]
        })
    }
    fn help(&self) -> &'static [(&'static str, &'static str)] {
        &[("r", "Refresh report"), ("i", "Inspect frame and query")]
    }
}

fn encoded(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
fn all_dashboard_entries(data: &Value) -> Vec<(String, &Value)> {
    if let Some(panels) = data["dashboard"]["panels"].as_object() {
        panels.iter().map(|(id, panel)| (id.clone(), panel)).collect()
    } else {
        data["dashboard"]["panels"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(at, panel)| {
                (
                    panel["id"].as_str().map_or_else(|| at.to_string(), str::to_string),
                    panel,
                )
            })
            .collect()
    }
}
fn dashboard_entries<'a>(data: &'a Value, state: &ComponentState) -> Vec<(String, &'a Value)> {
    let entries = all_dashboard_entries(data);
    let layout = &data["dashboard"]["layout"];
    if layout["type"] != "tabs" {
        return entries;
    }
    let Some(items) = layout["items"].as_array().or_else(|| layout["children"].as_array()) else {
        return entries;
    };
    let Some(tab) = items.get(state.choosing.unwrap_or(0).min(items.len().saturating_sub(1))) else {
        return entries;
    };
    fn ids(node: &Value, output: &mut BTreeSet<String>) {
        if let Some(id) = node["panelId"].as_str() {
            output.insert(id.into());
        }
        if let Some(id) = node.as_str() {
            output.insert(id.into());
        }
        if let Some(layout) = node.get("layout") {
            ids(layout, output);
        }
        for child in node["items"]
            .as_array()
            .or_else(|| node["children"].as_array())
            .into_iter()
            .flatten()
        {
            ids(child, output);
        }
    }
    let mut visible = BTreeSet::new();
    ids(tab, &mut visible);
    entries.into_iter().filter(|(id, _)| visible.contains(id)).collect()
}
fn dashboard_url(dashboard: &Value, action: &str, panel_id: Option<&str>, mode: Option<&str>) -> String {
    let mut pairs = vec![("definitionId".to_string(), text_of(dashboard.get("definitionId")))];
    for key in ["from", "thru"] {
        if let Some(value) = dashboard["absoluteTime"][key].as_str() {
            pairs.push((key.into(), value.into()));
        }
    }
    if action == "exportPanel" {
        pairs.push(("variables".into(), dashboard["variableValues"].to_string()));
    } else {
        for (key, value) in dashboard["variableValues"].as_object().into_iter().flatten() {
            for item in value
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(std::slice::from_ref(value))
            {
                pairs.push((format!("var-{key}"), text_of(Some(item))));
            }
        }
    }
    if let Some(panel) = panel_id {
        pairs.push(("panelId".into(), panel.into()));
    }
    if let Some(mode) = mode {
        pairs.push(("renderMode".into(), mode.into()));
    }
    let path = if action.is_empty() {
        "/apps/dashboards/Dashboard".to_string()
    } else {
        format!("/apps/dashboards/Dashboard/{action}")
    };
    format!(
        "{path}?{}",
        pairs
            .into_iter()
            .map(|(key, value)| format!("{}={}", encoded(&key), encoded(&value)))
            .collect::<Vec<_>>()
            .join("&")
    )
}
fn span_stamp(span: &Value) -> u64 {
    integer(&span["start_nano"]).unwrap_or_else(|| {
        integer(&span["start"])
            .unwrap_or_else(|| {
                span["start"]
                    .as_str()
                    .and_then(|text| text.parse::<jiff::Timestamp>().ok())
                    .and_then(|stamp| u64::try_from(stamp.as_millisecond()).ok())
                    .unwrap_or(0)
            })
            .saturating_mul(1_000_000)
    })
}

impl Component for ReportDashboard {
    fn name(&self) -> &'static str {
        "report-dashboard"
    }
    fn source(&self, props: &Value, state: &ComponentState) -> Option<(String, BTreeMap<String, String>)> {
        source_of(props, state)
    }
    fn lines(&self, props: &Value, state: &ComponentState, width: u16, _wide: bool) -> Vec<Line> {
        if let Some(line) = waiting(state) {
            return vec![line];
        }
        let data = state.data.as_ref().unwrap_or(props);
        let entries = dashboard_entries(data, state);
        let mut lines = Vec::new();
        if data["dashboard"]["layout"]["type"] == "tabs" {
            let items = &data["dashboard"]["layout"]["items"];
            let at = state.choosing.unwrap_or(0);
            lines.push(vec![Span::new(
                format!(
                    "Tab {}: {} (←/→ changes tabs)",
                    at + 1,
                    text_of(items.get(at).and_then(|item| item.get("title")))
                ),
                Role::Primary,
            )]);
        }
        for (at, (id, panel)) in entries.iter().enumerate() {
            if at == state.at {
                lines.push(vec![Span::new(
                    "▶ Selected panel; i inspects, c/x exports CSV/XLSX",
                    Role::Muted,
                )]);
            }
            let frame = data["frames"][id].get(0).unwrap_or(&Value::Null);
            lines.extend(panel_lines(frame, panel, width));
            if at == state.at && state.closed.contains("__report_inspector") {
                lines.extend(
                    serde_json::to_string_pretty(&serde_json::json!({"panelId":id,"panel":panel,"frame":frame}))
                        .unwrap_or_default()
                        .lines()
                        .take(300)
                        .map(|line| vec![Span::new(line, Role::Muted)]),
                );
            }
            lines.push(Vec::new());
        }
        if lines.is_empty() {
            lines.push(vec![Span::new("No panels", Role::Muted)]);
        }
        lines
    }
    fn key(&self, props: &Value, state: &mut ComponentState, press: &KeyPress, _wide: bool) -> Option<Vec<Act>> {
        if press.is_plain() {
            let data = state.data.as_ref().unwrap_or(props);
            let entries = dashboard_entries(data, state);
            let dashboard = &data["dashboard"];
            if !state.closed.contains("__time_prefix")
                && matches!(press.key, Key::Left | Key::Right)
                && dashboard["layout"]["type"] == "tabs"
            {
                let count = dashboard["layout"]["items"]
                    .as_array()
                    .or_else(|| dashboard["layout"]["children"].as_array())
                    .map_or(0, Vec::len);
                if count > 0 {
                    let at = state.choosing.unwrap_or(0);
                    state.choosing = Some(if press.key == Key::Left {
                        (at + count - 1) % count
                    } else {
                        (at + 1) % count
                    });
                    state.at = 0;
                    return Some(Vec::new());
                }
            }
            match press.key {
                Key::Up => {
                    state.at = state.at.saturating_sub(1);
                    return Some(Vec::new());
                }
                Key::Down => {
                    state.at = (state.at + 1).min(entries.len().saturating_sub(1));
                    return Some(Vec::new());
                }
                Key::Char('i') => {
                    if !state.closed.remove("__report_inspector") {
                        state.closed.insert("__report_inspector".into());
                    }
                    return Some(Vec::new());
                }
                Key::Char('e') => {
                    return Some(vec![Act::Follow(format!(
                        "/apps/dashboards/Definition?definitionId={}",
                        encoded(dashboard["definitionId"].as_str().unwrap_or(""))
                    ))]);
                }
                Key::Char('c') | Key::Char('x') => {
                    if let Some((id, _)) = entries.get(state.at) {
                        return Some(vec![Act::DownloadReport(dashboard_url(
                            dashboard,
                            "exportPanel",
                            Some(id),
                            Some(if press.key == Key::Char('c') { "csv" } else { "xlsx" }),
                        ))]);
                    }
                }
                Key::Enter | Key::Char('v') => {
                    if let Some((id, _)) = entries.get(state.at) {
                        return Some(vec![Act::Follow(dashboard_url(dashboard, "", Some(id), None))]);
                    }
                }
                Key::Char('t') => {
                    state.closed.insert("__time_prefix".into());
                    return Some(Vec::new());
                }
                _ => {}
            }
            if state.closed.remove("__time_prefix") {
                let from = dashboard["absoluteTime"]["from"]
                    .as_str()
                    .and_then(|text| text.parse::<jiff::Timestamp>().ok());
                let thru = dashboard["absoluteTime"]["thru"]
                    .as_str()
                    .and_then(|text| text.parse::<jiff::Timestamp>().ok());
                if let (Some(from), Some(thru)) = (from, thru) {
                    let width = thru.as_millisecond().saturating_sub(from.as_millisecond());
                    let offsets = match press.key {
                        Key::Left => Some((-width, -width)),
                        Key::Right => Some((width, width)),
                        Key::Char('-') => Some((-width / 2, width / 2)),
                        _ => None,
                    };
                    if let Some((left, right)) = offsets {
                        let mut next = dashboard.clone();
                        next["absoluteTime"]["from"] = Value::String(
                            jiff::Timestamp::from_millisecond(from.as_millisecond().saturating_add(left))
                                .ok()?
                                .to_string(),
                        );
                        next["absoluteTime"]["thru"] = Value::String(
                            jiff::Timestamp::from_millisecond(thru.as_millisecond().saturating_add(right))
                                .ok()?
                                .to_string(),
                        );
                        return Some(vec![Act::Follow(dashboard_url(&next, "", None, None))]);
                    }
                }
            }
        }
        (press.key == Key::Char('r') && press.is_plain()).then(|| {
            vec![if source_of(props, state).is_some() {
                Act::Reload
            } else {
                Act::ReloadScreen
            }]
        })
    }
    fn help(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("↑/↓", "Select panel"),
            ("←/→", "Choose dashboard tab"),
            ("i", "Inspect selected panel"),
            ("v/Enter", "View panel"),
            ("e", "Edit definition"),
            ("c/x", "Export CSV/XLSX"),
            ("t-", "Zoom out"),
            ("t←/t→", "Shift time range"),
            ("r", "Refresh dashboard"),
        ]
    }
}

impl Component for TraceWaterfall {
    fn name(&self) -> &'static str {
        "trace-waterfall"
    }
    fn source(&self, props: &Value, state: &ComponentState) -> Option<(String, BTreeMap<String, String>)> {
        source_of(props, state)
    }
    fn lines(&self, props: &Value, state: &ComponentState, width: u16, _wide: bool) -> Vec<Line> {
        if let Some(line) = waiting(state) {
            return vec![line];
        }
        let data = state.data.as_ref().unwrap_or(props);
        let mut spans: Vec<&Value> = data["spans"].as_array().into_iter().flatten().collect();
        if spans.is_empty() {
            return vec![vec![Span::new("No spans; telemetry may be missing", Role::Muted)]];
        }
        spans.sort_by_key(|span| span_stamp(span));
        let first = span_stamp(spans[0]);
        let last = spans
            .iter()
            .map(|span| span_stamp(span).saturating_add(integer(&span["duration_nano"]).unwrap_or(0)))
            .max()
            .unwrap_or(first);
        let duration = last.saturating_sub(first).max(1);
        let chart_width = usize::from(width).saturating_sub(42).min(80);
        let by_id: BTreeMap<&str, &Value> = spans
            .iter()
            .filter_map(|span| span["span_id"].as_str().map(|id| (id, *span)))
            .collect();
        let mut lines: Vec<Line> = spans
            .iter()
            .map(|span| {
                let mut seen = BTreeSet::new();
                let mut parent = span["parent_span_id"].as_str().unwrap_or("");
                let mut depth = 0;
                while let Some(ancestor) = by_id.get(parent) {
                    if !seen.insert(parent) || depth >= 16 {
                        break;
                    }
                    depth += 1;
                    parent = ancestor["parent_span_id"].as_str().unwrap_or("");
                }
                let elapsed = integer(&span["duration_nano"]).unwrap_or(0);
                let offset = ((u128::from(span_stamp(span).saturating_sub(first)) * chart_width as u128)
                    / u128::from(duration)) as usize;
                let length = ((u128::from(elapsed) * chart_width as u128) / u128::from(duration)) as usize;
                let title = fit(&format!("{}{}", " ".repeat(depth * 2), prop(span, "name", "span")), 28);
                #[expect(clippy::cast_precision_loss)]
                let millis = elapsed as f64 / 1_000_000.0;
                vec![
                    Span::new(format!("{title} {millis:>8.2}ms "), Role::Text),
                    Span::new(
                        format!("{}{}", " ".repeat(offset), "━".repeat(length.max(1))),
                        if span["status_code"] == "ERROR" {
                            Role::Negative
                        } else {
                            Role::Primary
                        },
                    ),
                ]
            })
            .collect();
        if state.closed.contains("__report_inspector") {
            if let Some(span) = spans.get(state.at) {
                lines.extend(
                    serde_json::to_string_pretty(span)
                        .unwrap_or_default()
                        .lines()
                        .map(|line| vec![Span::new(line, Role::Muted)]),
                );
            }
        }
        lines
    }
    fn key(&self, props: &Value, state: &mut ComponentState, press: &KeyPress, _wide: bool) -> Option<Vec<Act>> {
        if press.is_plain() {
            let data = state.data.as_ref().unwrap_or(props);
            let count = data["spans"].as_array().map_or(0, Vec::len);
            match press.key {
                Key::Up => {
                    state.at = state.at.saturating_sub(1);
                    return Some(Vec::new());
                }
                Key::Down => {
                    state.at = (state.at + 1).min(count.saturating_sub(1));
                    return Some(Vec::new());
                }
                Key::Enter | Key::Char('i') => {
                    if !state.closed.remove("__report_inspector") {
                        state.closed.insert("__report_inspector".into());
                    }
                    return Some(Vec::new());
                }
                _ => {}
            }
        }
        (press.key == Key::Char('r') && press.is_plain()).then(|| {
            vec![if source_of(props, state).is_some() {
                Act::Reload
            } else {
                Act::ReloadScreen
            }]
        })
    }
    fn help(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("↑/↓", "Select span"),
            ("i/Enter", "Inspect span attributes/events"),
            ("r", "Refresh trace"),
        ]
    }
}

impl Component for LogTail {
    fn name(&self) -> &'static str {
        "log-tail"
    }
    fn source(&self, props: &Value, state: &ComponentState) -> Option<(String, BTreeMap<String, String>)> {
        let (url, mut parameters) = source_of(props, state)?;
        if let Some(cursor) = state
            .data
            .as_ref()
            .and_then(|data| data.get("nextCursor"))
            .filter(|cursor| !cursor.is_null())
        {
            parameters.insert("cursor".into(), cursor.to_string());
        }
        Some((url, parameters))
    }
    fn receive(&self, _props: &Value, state: &mut ComponentState, mut value: Value) {
        let previous = state.data.as_ref();
        let mut hits = previous
            .and_then(|data| data["hits"].as_array())
            .cloned()
            .unwrap_or_default();
        let mut ids: BTreeSet<String> = hits
            .iter()
            .map(|hit| format!("{}:{}", text_of(hit.get("documentId")), text_of(hit.get("id"))))
            .collect();
        for hit in value["hits"].as_array().into_iter().flatten() {
            if ids.insert(format!("{}:{}", text_of(hit.get("documentId")), text_of(hit.get("id")))) {
                hits.push(hit.clone());
            }
        }
        let dropped = hits.len().saturating_sub(500);
        if dropped > 0 {
            hits.drain(..dropped);
        }
        if value["nextCursor"].is_null() {
            value["nextCursor"] = previous.map(|data| data["nextCursor"].clone()).unwrap_or(Value::Null);
        }
        value["removed"] =
            Value::from(previous.and_then(|data| data["removed"].as_u64()).unwrap_or(0) + dropped as u64);
        value["hits"] = Value::Array(hits);
        state.data = Some(value);
    }
    fn poll_millis(&self, _props: &Value, state: &ComponentState) -> Option<u64> {
        state.closed.contains("__tail_running").then_some(2000)
    }
    fn lines(&self, props: &Value, state: &ComponentState, width: u16, _wide: bool) -> Vec<Line> {
        if let Some(line) = waiting(state) {
            return vec![line];
        }
        let data = state.data.as_ref().unwrap_or(props);
        let hits = data["hits"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if hits.is_empty() {
            return vec![vec![Span::new(
                "No entries; r refreshes. Missing telemetry does not establish health.",
                Role::Muted,
            )]];
        }
        let mut lines = vec![vec![Span::new(
            if state.closed.contains("__tail_running") {
                "Live tail running; p pauses"
            } else {
                "Live tail paused; p resumes"
            },
            Role::Muted,
        )]];
        lines.extend(hits.iter().enumerate().map(|(at, hit)| {
            let body = &hit["body"];
            let role = if body["severity_number"].as_i64().unwrap_or(0) >= 17 {
                Role::Negative
            } else {
                Role::Text
            };
            vec![Span::new(
                fit(
                    &format!(
                        "{}{} {} {}",
                        if at == state.at { "▶ " } else { "  " },
                        text_of(body.get("timestamp")),
                        prop(body, "severity_text", ""),
                        prop(body, "body", prop(body, "name", ""))
                    ),
                    usize::from(width),
                ),
                role,
            )]
        }));
        if state.closed.contains("__report_inspector") {
            if let Some(hit) = hits.get(state.at) {
                lines.extend(
                    serde_json::to_string_pretty(hit)
                        .unwrap_or_default()
                        .lines()
                        .map(|line| vec![Span::new(line, Role::Text)]),
                );
            }
        }
        lines
    }
    fn key(&self, props: &Value, state: &mut ComponentState, press: &KeyPress, _wide: bool) -> Option<Vec<Act>> {
        if press.is_plain() {
            match press.key {
                Key::Char('p') => {
                    if state.closed.remove("__tail_running") {
                        return Some(Vec::new());
                    }
                    state.closed.insert("__tail_running".into());
                    return Some(vec![Act::Reload]);
                }
                Key::Char('i') | Key::Enter => {
                    if !state.closed.remove("__report_inspector") {
                        state.closed.insert("__report_inspector".into());
                    }
                    return Some(Vec::new());
                }
                Key::Down => {
                    state.at = (state.at + 1).min(
                        state
                            .data
                            .as_ref()
                            .and_then(|data| data["hits"].as_array())
                            .map_or(0, |hits| hits.len().saturating_sub(1)),
                    );
                    return Some(Vec::new());
                }
                Key::Up => {
                    state.at = state.at.saturating_sub(1);
                    return Some(Vec::new());
                }
                Key::Char('t') => {
                    if let Some(trace) = state
                        .data
                        .as_ref()
                        .and_then(|data| data["hits"].get(state.at))
                        .and_then(|hit| hit["body"]["trace_id"].as_str())
                        .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
                    {
                        return Some(vec![Act::Follow(format!("/apps/monitor/TraceDetail?traceId={trace}"))]);
                    }
                }
                _ => {}
            }
        }
        (press.key == Key::Char('r') && press.is_plain()).then(|| {
            vec![if source_of(props, state).is_some() {
                Act::Reload
            } else {
                Act::ReloadScreen
            }]
        })
    }
    fn help(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("r", "Read latest telemetry"),
            ("p", "Pause/resume live tail"),
            ("↑/↓", "Select entry"),
            ("i/Enter", "Inspect entry"),
            ("t", "Open trace"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::line_text;
    use serde_json::json;
    #[test]
    fn frames_show_null_as_missing_and_share_the_browser_numbers() {
        let props = json!({"panel":{"title":"Latency","visualization":"table"},"frame":{"fields":[{"name":"duration","type":"number","unit":"ns","values":[1250000,null]}]}});
        let lines: Vec<_> = ReportPanel
            .lines(&props, &ComponentState::default(), 60, false)
            .iter()
            .map(line_text)
            .collect();
        assert!(lines.iter().any(|line| line.contains("1.25 ms")));
        assert!(lines.iter().any(|line| line.contains('—')));
    }
    #[test]
    fn waterfall_preserves_nanosecond_offsets_and_parent_depth() {
        let props = json!({"spans":[{"span_id":"root","name":"request","start_nano":"1800000000000000000","duration_nano":10000000},
            {"span_id":"child","parent_span_id":"root","name":"service","start_nano":"1800000000005000000","duration_nano":1000000}]});
        let lines: Vec<_> = TraceWaterfall
            .lines(&props, &ComponentState::default(), 100, false)
            .iter()
            .map(line_text)
            .collect();
        assert!(lines[1].starts_with("  service"));
        assert!(lines[0].contains("10.00ms"));
        assert!(lines[1].contains("1.00ms"));
        assert!(
            lines[1].find('━') > lines[0].find('━'),
            "the child retains its offset when nanoseconds arrive as strings"
        );
    }
    #[test]
    fn field_overrides_and_inline_refresh_are_real() {
        let props = json!({"panel":{"title":"Latency","fieldConfig":{"defaults":{"decimals":1},"overrides":[{"match":{"name":"latency"},"config":{"unit":"s","noValue":"Missing"}}]}},"frame":{"fields":[{"name":"latency","type":"number","values":[1.25,null]}]}});
        let lines: Vec<_> = ReportPanel
            .lines(&props, &ComponentState::default(), 60, false)
            .iter()
            .map(line_text)
            .collect();
        assert!(lines.iter().any(|line| line.contains("1.2 s")));
        assert!(lines.iter().any(|line| line.contains("Missing")));
        let mut state = ComponentState::default();
        assert_eq!(
            ReportPanel.key(&props, &mut state, &KeyPress::plain(Key::Char('r')), false),
            Some(vec![Act::ReloadScreen])
        );
        ReportPanel.key(&props, &mut state, &KeyPress::plain(Key::Char('i')), false);
        assert!(
            ReportPanel
                .lines(&props, &state, 60, false)
                .iter()
                .map(line_text)
                .any(|line| line.contains("fieldConfig"))
        );
    }
    #[test]
    fn dashboard_tabs_inspector_exports_and_time_keys_use_current_scope() {
        let props = json!({"dashboard":{"definitionId":"monitor-artifacts","absoluteTime":{"from":"2026-01-01T00:00:00Z","thru":"2026-01-02T00:00:00Z"},"variableValues":{"service":"ERP"},"layout":{"type":"tabs","items":[{"title":"First","layout":{"items":[{"panelId":"one"}]}},{"title":"Second","layout":{"items":[{"panelId":"two"}]}}]},"panels":{"one":{"title":"Panel one"},"two":{"title":"Panel two","queries":[{"datasetId":"otel_metrics"}]}}},"frames":{}});
        let mut state = ComponentState::default();
        let text = |state: &ComponentState| {
            ReportDashboard
                .lines(&props, state, 100, false)
                .iter()
                .map(line_text)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(text(&state).contains("Panel one"));
        assert!(!text(&state).contains("Panel two"));
        ReportDashboard.key(&props, &mut state, &KeyPress::plain(Key::Right), false);
        assert!(text(&state).contains("Panel two"));
        ReportDashboard.key(&props, &mut state, &KeyPress::plain(Key::Char('i')), false);
        assert!(text(&state).contains("otel_metrics"));
        let export = ReportDashboard
            .key(&props, &mut state, &KeyPress::plain(Key::Char('x')), false)
            .unwrap();
        assert!(
            matches!(&export[0],Act::DownloadReport(url) if url.contains("panelId=two") && url.contains("renderMode=xlsx") && url.contains("ERP"))
        );
        ReportDashboard.key(&props, &mut state, &KeyPress::plain(Key::Char('t')), false);
        let shifted = ReportDashboard
            .key(&props, &mut state, &KeyPress::plain(Key::Right), false)
            .unwrap();
        assert!(
            matches!(&shifted[0],Act::Follow(url) if url.contains("from=2026-01-02") && url.contains("var-service=ERP"))
        );
    }
    #[test]
    fn live_tail_merges_cursor_pages_bounds_rows_and_pauses() {
        let props = json!({"source":"/tail"});
        let mut state = ComponentState::default();
        LogTail.receive(
            &props,
            &mut state,
            json!({"hits":[{"documentId":"otel_logs","id":"first","body":{"body":"first"}}],"nextCursor":{"id":1}}),
        );
        assert_eq!(LogTail.poll_millis(&props, &state), None);
        assert_eq!(
            LogTail.key(&props, &mut state, &KeyPress::plain(Key::Char('p')), false),
            Some(vec![Act::Reload])
        );
        assert_eq!(LogTail.poll_millis(&props, &state), Some(2000));
        assert_eq!(LogTail.source(&props, &state).unwrap().1["cursor"], "{\"id\":1}");
        LogTail.receive(&props,&mut state,json!({"hits":[{"documentId":"otel_logs","id":"first"},{"documentId":"otel_logs","id":"second"}],"nextCursor":{"id":2}}));
        assert_eq!(state.data.as_ref().unwrap()["hits"].as_array().unwrap().len(), 2);
        let hits: Vec<_> = (0..600)
            .map(|at| json!({"documentId":"otel_logs","id":at.to_string()}))
            .collect();
        LogTail.receive(&props, &mut state, json!({"hits":hits,"nextCursor":null}));
        assert_eq!(state.data.as_ref().unwrap()["hits"].as_array().unwrap().len(), 500);
        assert_eq!(state.data.as_ref().unwrap()["nextCursor"], json!({"id":2}));
        LogTail.key(&props, &mut state, &KeyPress::plain(Key::Char('p')), false);
        assert_eq!(LogTail.poll_millis(&props, &state), None);
    }
    #[test]
    fn span_inspector_preserves_attributes_and_events() {
        let props = json!({"spans":[{"span_id":"child","name":"child","start_nano":"1800000000000000010","attributes":{"http.route":"/ERP"},"events":[{"name":"exception"}]},{"span_id":"root","name":"root","start_nano":"1800000000000000000"}]});
        let mut state = ComponentState::default();
        TraceWaterfall.key(&props, &mut state, &KeyPress::plain(Key::Down), false);
        TraceWaterfall.key(&props, &mut state, &KeyPress::plain(Key::Char('i')), false);
        let text = TraceWaterfall
            .lines(&props, &state, 100, false)
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("http.route") && text.contains("exception"));
    }
    #[test]
    fn text_panels_do_not_require_frames_and_bytes_use_the_browser_scale() {
        let lines = panel_lines(
            &json!({"fields":[]}),
            &json!({"title":"Notes","visualization":"text","text":"Release notes\nKeep watching latency"}),
            80,
        );
        let text = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(text.contains("Release notes") && text.contains("Keep watching latency"));
        assert!(!text.contains("No data"));
        assert_eq!(
            formatted(&json!(2048), &json!({"type":"number","unit":"bytes"})),
            "2 KiB"
        );
        assert_eq!(
            formatted(&json!(1536), &json!({"type":"number","unit":"bytes"})),
            "1.50 KiB"
        );
        let fields = configured_fields(
            &json!({"fields":[{"name":"value","unit":"s"}]}),
            &json!({"fieldConfig":{"defaults":{"unit":"ms"}}}),
        );
        assert_eq!(fields[0]["unit"], "s");
    }
    #[test]
    fn partial_and_subtotal_metadata_are_not_numeric_chart_series() {
        let frame = json!({"fields":[{"name":"partial","type":"boolean","values":[true]},{"name":"__level","type":"number","values":[0]},{"name":"__subtotal","type":"number","values":[1]},{"name":"time","type":"time","values":[1000]},{"name":"requests","type":"number","values":[3]}]});
        let fields = configured_fields(&frame, &json!({}));
        let series: Vec<_> = fields
            .iter()
            .filter(|field| field["type"] == "number" && !is_metadata(field))
            .map(|field| field["name"].as_str().unwrap())
            .collect();
        assert_eq!(series, ["requests"]);
        let lines = panel_lines(&frame, &json!({"visualization":"stat"}), 80)
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lines.contains('3'));
        assert!(!lines.contains("0.00"));
    }
    #[test]
    fn gaps_are_missing_in_sparklines() {
        assert_eq!(sparkline(&[json!(1), Value::Null, json!(3)], 3), "▁ █");
    }
}
