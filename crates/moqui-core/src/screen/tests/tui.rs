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

//! The `tui` render mode writes one JSON document: an envelope around a root array, and each section is an array.

use std::collections::BTreeMap;

use moqui_lib::object::Value;
use serde_json::{json, Value as Json};

use crate::context::ExecutionContextFactory;
use crate::screen::{make_render, RenderOutput};

fn render(label: &str, xml: &str, parameters: BTreeMap<String, Value>) -> RenderOutput {
    let path = std::env::temp_dir().join(format!("moqui-tui-{}-{label}.xml", std::process::id()));
    std::fs::write(&path, xml).unwrap();
    let context = ExecutionContextFactory::test_context();
    let output = make_render(context.factory())
        .root_screen(path.to_string_lossy().into_owned())
        .render_mode("tui")
        .parameters(parameters)
        .render()
        .unwrap_or_else(|err| panic!("{label}: {err}"));
    let _ = std::fs::remove_file(&path);
    output
}

fn document(output: &RenderOutput) -> Json {
    let text = output.body_text();
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err}: {text}"))
}

fn objects<'a>(value: &'a Json, found: &mut Vec<&'a serde_json::Map<String, Json>>) {
    match value {
        Json::Array(items) => {
            for item in items {
                objects(item, found);
            }
        }
        Json::Object(map) => {
            found.push(map);
            for item in map.values() {
                objects(item, found);
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::String(_) => {}
    }
}

fn nodes(doc: &Json) -> Vec<&serde_json::Map<String, Json>> {
    let mut found = Vec::new();
    objects(doc, &mut found);
    found
}

fn section<'a>(doc: &'a Json, name: &str) -> &'a serde_json::Map<String, Json> {
    nodes(doc)
        .into_iter()
        .find(|node| {
            node.get("t").and_then(Json::as_str) == Some("section")
                && node.get("name").and_then(Json::as_str) == Some(name)
        })
        .unwrap_or_else(|| panic!("no section {name} in {doc}"))
}

#[test]
fn an_empty_section_a_failed_section_and_a_section_iterate_are_one_document() {
    let output = render(
        "sections",
        r#"<screen require-authentication="false" default-menu-title="Enums"><actions><script>
            ec.message.addMessage("saved ok")
            ec.message.addMessage("watch this", "danger")
        </script></actions><widgets>
            <label text="before"/>
            <section name="Empty"><widgets/></section>
            <section-iterate name="Vacant" list="[]" entry="row"><widgets><label text="nope"/></widgets></section-iterate>
            <section-iterate name="None" list="['a','b']" entry="row"><widgets/></section-iterate>
            <section-iterate name="Each" list="['a','b','c']" entry="row"><widgets><label text="${row}"/></widgets></section-iterate>
            <section name="Broken"><widgets><label text="inside-broken"/>
                <render-mode><text type="tui"><![CDATA[${(() => { throw new Error('boom { brace }') })()}]]></text></render-mode>
            </widgets></section>
            <section name="Checked"><actions><script>
                ec.message.addError("plain problem")
                ec.message.addValidationError("EditForm", "name", "", "Name is required")
            </script></actions><widgets><label text="inside-checked"/></widgets></section>
            <label text="after"/>
        </widgets></screen>"#,
        BTreeMap::new(),
    );
    assert_eq!(output.status, 200);
    assert!(
        output.content_type().unwrap_or("").starts_with("application/json"),
        "{}",
        output.content_type().unwrap_or("")
    );
    let doc = document(&output);
    assert_eq!(doc["v"], json!(1));
    assert_eq!(doc["screen"]["path"], json!("/"));
    assert_eq!(doc["screen"]["pathList"], json!([]));
    assert_eq!(doc["screen"]["title"], json!("Enums"));
    assert_eq!(doc["screen"]["standalone"], json!(0));
    assert_eq!(doc["focus"], json!(""));
    assert_eq!(doc["messages"]["info"], json!(["saved ok"]));
    assert_eq!(doc["messages"]["errors"], json!(["watch this", "plain problem"]));
    assert_eq!(
        doc["messages"]["validationErrors"],
        json!([{"form": "EditForm", "field": "name", "message": "Name is required"}])
    );
    let body = output.body_text();
    assert!(!body.contains("inside-broken"), "{body}");
    assert!(!body.contains("inside-checked"), "{body}");
    assert!(body.contains("before") && body.contains("after"), "{body}");
    assert!(section(&doc, "Empty")["c"].as_array().unwrap().is_empty(), "{doc}");
    assert!(section(&doc, "Vacant")["c"].as_array().unwrap().is_empty(), "{doc}");
    assert!(section(&doc, "None")["c"].as_array().unwrap().is_empty(), "{doc}");
    let each = section(&doc, "Each")["c"].as_array().unwrap();
    assert_eq!(
        each.iter()
            .map(|node| node["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(!body.contains("]["), "{body}");
    let broken = section(&doc, "Broken")["c"].as_array().unwrap();
    assert_eq!(broken[0]["t"], json!("error"));
    assert!(broken[0]["text"].as_str().unwrap().contains("boom"), "{broken:?}");
    assert!(broken[0]["text"].as_str().unwrap().contains("brace"), "{broken:?}");
    let checked = section(&doc, "Checked")["c"].as_array().unwrap();
    assert_eq!(checked[0]["t"], json!("error"));
    assert!(
        checked[0]["text"].as_str().unwrap().contains("Name is required"),
        "{checked:?}"
    );
    let model = moqui_tui::ScreenModel::from_json(&body).unwrap_or_else(|err| panic!("{err}: {body}"));
    assert_eq!(model.v, 1);
    assert_eq!(model.screen.title, "Enums");
    assert_eq!(model.messages.info, ["saved ok"]);
    assert_eq!(model.messages.validation_errors[0].message, "Name is required");
}

#[test]
fn a_password_a_credential_parameter_and_a_control_sequence_are_left_out() {
    let mut parameters = BTreeMap::new();
    parameters.insert("password".to_string(), Value::string("s3cret-value"));
    parameters.insert("keep".to_string(), Value::string("kept-visible"));
    parameters.insert("moquiSessionToken".to_string(), Value::string("token-value"));
    let output = render(
        "secret",
        r#"<screen require-authentication="false" default-menu-title="Secret"><actions><script>
            bad = "pre\u001b]52;\u0080post"
        </script></actions><widgets>
            <label text="${bad}"/>
            <form-single name="F"><field name="pw"><default-field><password default-value="s3cret-value"/></default-field></field></form-single>
        </widgets></screen>"#,
        parameters,
    );
    let body = output.body_text();
    assert!(!body.contains("s3cret-value"), "{body}");
    assert!(!body.contains("token-value"), "{body}");
    assert!(!body.as_bytes().contains(&0x1b), "{body}");
    let doc = document(&output);
    assert_eq!(doc["parameters"]["keep"], json!("kept-visible"));
    assert!(doc["parameters"].get("password").is_none(), "{doc}");
    assert!(doc["parameters"].get("moquiSessionToken").is_none(), "{doc}");
    let text = nodes(&doc)
        .into_iter()
        .find(|node| node.get("t").and_then(Json::as_str) == Some("text"))
        .unwrap();
    assert_eq!(text["text"], json!("pre]52;post"));
    let field = nodes(&doc)
        .into_iter()
        .find(|node| node.get("t").and_then(Json::as_str) == Some("field"))
        .unwrap();
    assert_eq!(field["w"]["kind"], json!("password"));
    assert!(field["w"].get("value").is_none(), "{field:?}");
    let model = moqui_tui::ScreenModel::from_json(&body).unwrap_or_else(|err| panic!("{err}: {body}"));
    assert_eq!(model.parameters.get("keep").map(String::as_str), Some("kept-visible"));
    assert!(!model.parameters.contains_key("password"));
}

#[test]
fn a_dialog_a_dependent_drop_down_a_list_and_a_tui_text_are_one_document() {
    let output = render(
        "edit",
        r#"<screen require-authentication="false" default-menu-title="Edit"><actions><script>
            rows = [{id:'r1', title:'One'}]
        </script></actions><widgets>
            <container-dialog id="New" button-text="New" title="Create">
                <label text="inside"/>
                <form-single name="NewForm" transition="create" focus-field="name" background-hide-id="New" submit-message="Saved">
                    <field name="name"><default-field title="Name"><text-line/></default-field></field>
                    <field name="pw"><default-field><password default-value="s3cret-value"/></default-field></field>
                </form-single>
            </container-dialog>
            <form-single name="Find">
                <field name="countryGeoId"><default-field title="Country">
                    <drop-down><option key="US" text="United States"/></drop-down>
                </default-field></field>
                <field name="stateGeoId"><default-field title="State">
                    <drop-down><dynamic-options transition="getGeoCountryStates" value-field="geoId" label-field="geoName">
                        <depends-on field="countryGeoId"/>
                    </dynamic-options></drop-down>
                </default-field></field>
            </form-single>
            <form-list name="Rows" list="rows" id-field="id">
                <field name="id"><default-field><display/></default-field></field>
                <field name="title">
                    <header-field title="Title"><text-find/></header-field>
                    <default-field><link url="/open" text="Open" link-type="anchor" url-type="plain"/></default-field>
                </field>
            </form-list>
            <text type="tui"><![CDATA[[{"t":"text","text":"from-tui"}]]]></text>
            <text type="tui">this is not json</text>
            <render-mode><text type="qvt">only qvt</text></render-mode>
        </widgets></screen>"#,
        BTreeMap::new(),
    );
    let body = output.body_text();
    assert!(!body.contains("s3cret-value"), "{body}");
    assert!(!body.contains("is not written yet"), "{body}");
    let doc = document(&output);
    assert_eq!(doc["focus"], json!("NewForm.name"));
    let dialog = nodes(&doc)
        .into_iter()
        .find(|node| node.get("t").and_then(Json::as_str) == Some("dialog"))
        .unwrap();
    assert_eq!(dialog["button"], json!("New"));
    assert_eq!(dialog["title"], json!("Create"));
    assert!(dialog["c"].to_string().contains("inside"), "{dialog:?}");
    let country = nodes(&doc)
        .into_iter()
        .find(|node| node.get("name").and_then(Json::as_str) == Some("countryGeoId"))
        .unwrap();
    assert_eq!(country["w"]["options"][0]["label"], json!("United States"));
    let state = nodes(&doc)
        .into_iter()
        .find(|node| node.get("name").and_then(Json::as_str) == Some("stateGeoId"))
        .unwrap();
    assert_eq!(state["w"]["dependsOn"], json!({"countryGeoId": "countryGeoId"}));
    assert!(state["w"].get("optionsParameters").is_none(), "{state:?}");
    assert!(state["w"]["options"].as_array().unwrap().is_empty(), "{state:?}");
    assert!(state["w"].get("optionsUrl").is_some(), "{state:?}");
    let list = nodes(&doc)
        .into_iter()
        .find(|node| node.get("t").and_then(Json::as_str) == Some("formList"))
        .unwrap();
    assert_eq!(list["rows"][0]["cells"][0], json!("r1"));
    assert_eq!(list["rows"][0]["cells"][1]["t"], json!("link"));
    assert!(
        list["rows"][0]["cells"][1]["url"].as_str().unwrap().contains("/open"),
        "{list:?}"
    );
    assert_eq!(list["headerFields"][0]["w"]["kind"], json!("textFind"));
    assert_eq!(list["paging"]["count"], json!(1));
    let texts: Vec<&str> = nodes(&doc)
        .iter()
        .filter_map(|node| node.get("text").and_then(Json::as_str))
        .collect();
    assert!(texts.contains(&"from-tui"), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|text| text.contains("is not JSON") && text.contains("inline")),
        "{texts:?}"
    );
    assert!(nodes(&doc).iter().any(|node| {
        node.get("t").and_then(Json::as_str) == Some("placeholder")
            && node
                .get("url")
                .and_then(Json::as_str)
                .is_some_and(|url| !url.is_empty())
    }));
    moqui_tui::ScreenModel::from_json(&body).unwrap_or_else(|err| panic!("{err}: {body}"));
}

fn ids(doc: &Json) -> Vec<String> {
    nodes(doc)
        .into_iter()
        .filter(|node| node.contains_key("t"))
        .filter_map(|node| node.get("id").and_then(Json::as_str).map(str::to_string))
        .collect()
}

fn first_of<'a>(doc: &'a Json, t: &str) -> &'a serde_json::Map<String, Json> {
    nodes(doc)
        .into_iter()
        .find(|node| node.get("t").and_then(Json::as_str) == Some(t))
        .unwrap_or_else(|| panic!("no {t} in {doc}"))
}

#[test]
fn links_keep_their_parameters_and_options_send_fixed_and_dependent_ones() {
    let output = render(
        "params",
        r#"<screen require-authentication="false"><actions><script>
            rows = [{enumId:'E1'}, {enumId:'E2'}]
        </script></actions><widgets>
            <link url="/apps/Detail" url-type="plain" text="Open"><parameter name="enumId" value="E0"/></link>
            <link url="/apps/Views" url-type="plain" text="Board" parameter-map="{view:'board'}"/>
            <form-list name="Rows" list="rows" id-field="enumId">
                <field name="enumId"><default-field>
                    <link url="/apps/Detail" url-type="plain" text="${enumId}"><parameter name="enumId"/></link>
                </default-field></field>
            </form-list>
            <form-single name="Geo">
                <field name="country"><default-field><text-line/></default-field></field>
                <field name="state"><default-field><drop-down>
                    <dynamic-options transition="/apps/states" value-field="geoId" label-field="geoName"
                            parameter-map="{geoTypeEnumId:'GEOT_STATE'}">
                        <depends-on field="country" parameter="countryGeoId"/>
                    </dynamic-options>
                </drop-down></default-field></field>
            </form-single>
        </widgets></screen>"#,
        BTreeMap::new(),
    );
    let doc = document(&output);
    let urls: Vec<&str> = nodes(&doc)
        .into_iter()
        .filter(|node| node.get("t").and_then(Json::as_str) == Some("link"))
        .filter_map(|node| node.get("url").and_then(Json::as_str))
        .collect();
    assert!(
        urls.iter().any(|url| url.ends_with("/apps/Detail?enumId=E0")),
        "{urls:?}"
    );
    assert!(
        urls.iter().any(|url| url.ends_with("/apps/Views?view=board")),
        "{urls:?}"
    );
    assert!(
        urls.iter().any(|url| url.ends_with("/apps/Detail?enumId=E2")),
        "{urls:?}"
    );
    let list = first_of(&doc, "formList");
    assert_eq!(list["rows"][0]["id"], json!("E1"));
    assert_eq!(list["rows"][1]["id"], json!("E2"));
    let state = nodes(&doc)
        .into_iter()
        .find(|node| node.get("name").and_then(Json::as_str) == Some("state"))
        .unwrap();
    assert_eq!(state["w"]["dependsOn"], json!({"countryGeoId": "country"}));
    assert_eq!(state["w"]["optionsParameters"], json!({"geoTypeEnumId": "GEOT_STATE"}));
    assert!(!state["w"]["optionsUrl"].as_str().unwrap().contains('?'), "{state:?}");
    let model = moqui_tui::ScreenModel::from_json(&output.body_text()).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(model.v, 1);
}

#[test]
fn texts_from_a_location_and_single_node_fragments_are_checked_before_they_are_written() {
    let dir = std::env::temp_dir().join(format!("moqui-tui-texts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tui = dir.join("Part.tui.json");
    std::fs::write(&tui, r#"{"t":"text","text":"from-file"}"#).unwrap();
    let plain = dir.join("Plain.tui.json");
    std::fs::write(&plain, r#"[{"t":"text","text":"plain ${not expanded}"}]"#).unwrap();
    let html = dir.join("Part.html");
    std::fs::write(&html, "<p>any-from-file</p>").unwrap();
    let xml = format!(
        r#"<screen require-authentication="false"><widgets>
            <render-mode><text type="tui" location="{}"/></render-mode>
            <render-mode><text type="tui" location="{}" template="false"/></render-mode>
            <render-mode><text location="{}"/></render-mode>
            <render-mode><text type="tui"><![CDATA[{{"t":"text","text":"single"}}]]></text></render-mode>
            <render-mode><text type="tui"><![CDATA[[1, 2]]]></text></render-mode>
        </widgets></screen>"#,
        tui.display(),
        plain.display(),
        html.display()
    );
    let output = render("texts", &xml, BTreeMap::new());
    let _ = std::fs::remove_dir_all(&dir);
    let doc = document(&output);
    let texts: Vec<&str> = nodes(&doc)
        .iter()
        .filter_map(|node| node.get("text").and_then(Json::as_str))
        .collect();
    assert!(texts.contains(&"from-file"), "{texts:?}");
    assert!(texts.contains(&"plain ${not expanded}"), "{texts:?}");
    assert!(texts.iter().any(|text| text.contains("any-from-file")), "{texts:?}");
    assert!(texts.contains(&"single"), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|text| text.contains("is not a node or an array of nodes")),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|text| text.contains("is not JSON")), "{texts:?}");
    moqui_tui::ScreenModel::from_json(&output.body_text()).unwrap_or_else(|err| panic!("{err}"));
}

#[test]
fn node_ids_are_unique_across_sections_includes_and_iterations() {
    let dir = std::env::temp_dir().join(format!("moqui-tui-ids-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let empty = dir.join("Empty.xml");
    std::fs::write(&empty, r#"<screen require-authentication="false"><widgets/></screen>"#).unwrap();
    let other = dir.join("Other.xml");
    std::fs::write(
        &other,
        r#"<screen require-authentication="false"><widgets>
            <section name="First"><widgets><label text="two"/></widgets></section>
        </widgets></screen>"#,
    )
    .unwrap();
    let xml = format!(
        r#"<screen require-authentication="false"><widgets>
            <section name="First"><widgets><label text="one"/></widgets></section>
            <section-include name="First" location="{}"/>
            <section-iterate name="Each" list="['a','b','c']" entry="row"><widgets>
                <label text="${{row}}"/>
                <container-box><box-header title="Box ${{row}}"/><box-body><label text="in"/></box-body></container-box>
            </widgets></section-iterate>
            <include-screen location="{}"/>
            <subscreens-panel id="Pop" type="popup"/>
            <form-list name="Cols" list="[{{a:1}}]" select-columns="true">
                <field name="a"><default-field><display/></default-field></field>
            </form-list>
        </widgets></screen>"#,
        other.display(),
        empty.display()
    );
    let output = render("ids", &xml, BTreeMap::new());
    let _ = std::fs::remove_dir_all(&dir);
    let body = output.body_text();
    let doc = document(&output);
    let found = ids(&doc);
    let mut seen = std::collections::HashSet::new();
    for id in &found {
        assert!(seen.insert(id), "{id} repeated in {found:?}");
    }
    assert!(found.iter().any(|id| id.ends_with("~2")), "{found:?}");
    let include = nodes(&doc)
        .into_iter()
        .find(|node| node.get("name").and_then(Json::as_str) == Some("include"))
        .unwrap_or_else(|| panic!("{doc}"));
    assert_eq!(include["c"], json!([]), "{doc}");
    // A popup panel draws no tabs but still holds the active level, as qvt's m-subscreens-active.
    let popup = first_of(&doc, "subscreens");
    assert_eq!(popup["kind"], json!("popup"));
    assert!(popup["c"].is_array(), "{popup:?}");
    assert!(
        nodes(&doc)
            .iter()
            .any(|node| node.get("t").and_then(Json::as_str) == Some("formList")),
        "{doc}"
    );
    moqui_tui::ScreenModel::from_json(&body).unwrap_or_else(|err| panic!("{err}: {body}"));
}

#[test]
fn every_phase_two_widget_and_region_is_a_node_the_client_reads() {
    let mut parameters = BTreeMap::new();
    parameters.insert("descr_op".to_string(), Value::string("begins"));
    parameters.insert("descr_not".to_string(), Value::string("Y"));
    parameters.insert("amount_from".to_string(), Value::string("5"));
    let output = render(
        "widgets",
        r#"<screen require-authentication="false"><widgets>
            <container-box><box-header title="Order"/><box-toolbar><label text="tool"/></box-toolbar>
                <box-body><label text="body" type="h3"/></box-body></container-box>
            <container-panel><panel-left size="200"><label text="side"/></panel-left><panel-center><label text="mid"/></panel-center></container-panel>
            <container-split side-width="240"><split-side><label text="list"/></split-side><split-main><label text="record"/></split-main></container-split>
            <form-single name="All" transition="save">
                <field name="note"><default-field tooltip="A note"><text-area rows="4" editor-type="md"/></default-field></field>
                <field name="body"><default-field><text-area editor-type="html"/></default-field></field>
                <field name="descr"><default-field><text-find/></default-field></field>
                <field name="when"><default-field><date-find type="date"/></default-field></field>
                <field name="period"><default-field><date-period/></default-field></field>
                <field name="amount"><default-field><range-find/></default-field></field>
                <field name="upload"><default-field><file multiple="true"/></default-field></field>
                <field name="shown"><default-field><display text="Shown"/></default-field></field>
                <field name="caption"><default-field><label text="Caption" type="h4"/></default-field></field>
                <field name="code"><default-field><text-line input-type="number" mask="999" size="6"/></default-field></field>
                <field name="types"><default-field><drop-down allow-multiple="true" allow-empty="true" show-not="true">
                    <option key="A" text="Alpha"/></drop-down></default-field></field>
                <field name="flag"><default-field><check><option key="Y" text="Yes"/></check></default-field></field>
                <field name="extra"><default-field>
                    <visible-when field="flag" value="Y"/><text-line/></default-field></field>
                <field name="go"><default-field><submit confirmation="Sure?"/></default-field></field>
            </form-single>
            <link url="/apps/x" url-type="plain" text="Do" btn-type="danger" icon="delete" badge="3" link-type="anchor-button"/>
        </widgets></screen>"#,
        parameters,
    );
    let body = output.body_text();
    let doc = document(&output);
    let field = |name: &str| {
        nodes(&doc)
            .into_iter()
            .find(|node| node.get("t").and_then(Json::as_str) == Some("field") && node["name"] == json!(name))
            .unwrap_or_else(|| panic!("no field {name} in {doc}"))
    };
    assert_eq!(field("note")["w"]["kind"], json!("textArea"));
    assert_eq!(field("note")["w"]["rows"], json!(4));
    assert_eq!(field("note")["tooltip"], json!("A note"));
    assert_eq!(field("body")["w"]["readOnly"], json!(true));
    assert_eq!(field("descr")["w"]["op"], json!("begins"));
    assert_eq!(field("descr")["w"]["not"], json!(true));
    assert_eq!(field("when")["w"]["kind"], json!("dateFind"));
    assert_eq!(field("period")["w"]["kind"], json!("datePeriod"));
    assert_eq!(field("amount")["w"]["from"], json!("5"));
    assert_eq!(field("upload")["w"]["multiple"], json!(true));
    assert_eq!(field("shown")["w"]["text"], json!("Shown"));
    assert_eq!(field("caption")["w"]["style"], json!("h4"));
    assert_eq!(field("code")["w"]["inputType"], json!("number"));
    assert_eq!(field("types")["w"]["multiple"], json!(true));
    assert_eq!(field("types")["w"]["allowEmpty"], json!(true));
    assert_eq!(field("flag")["w"]["options"][0]["label"], json!("Yes"));
    assert_eq!(field("extra")["visibleWhen"], json!({"field": "flag", "value": "Y"}));
    assert_eq!(field("go")["w"]["confirm"], json!("Sure?"));
    let regions: Vec<&str> = nodes(&doc)
        .into_iter()
        .filter(|node| node.get("t").and_then(Json::as_str) == Some("region"))
        .filter_map(|node| node["name"].as_str())
        .collect();
    assert_eq!(regions, ["toolbar", "left", "center", "side", "main"]);
    let left = nodes(&doc)
        .into_iter()
        .find(|node| node.get("name") == Some(&json!("left")))
        .unwrap();
    assert_eq!(left["size"], json!(200));
    assert_eq!(first_of(&doc, "split")["sideWidth"], json!(240));
    let link = first_of(&doc, "link");
    assert_eq!(link["btnType"], json!("danger"));
    assert_eq!(link["icon"], json!("delete"));
    assert_eq!(link["badge"], json!("3"));
    assert!(
        nodes(&doc)
            .iter()
            .any(|node| node.get("t") == Some(&json!("text")) && node.get("style") == Some(&json!("h3"))),
        "{doc}"
    );
    assert!(!body.contains("\"placeholder\""), "{body}");
    let model = moqui_tui::ScreenModel::from_json(&body).unwrap_or_else(|err| panic!("{err}: {body}"));
    let mut kinds = Vec::new();
    moqui_tui::walk(&model.c, &mut |node| {
        if let moqui_tui::Node::Field { w, .. } = node {
            kinds.push(format!("{w:?}").split([' ', '{']).next().unwrap_or("").to_string());
        }
    });
    assert!(kinds.contains(&"DatePeriod".to_string()), "{kinds:?}");
}
