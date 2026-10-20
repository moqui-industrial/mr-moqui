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
    assert_eq!(state["w"]["dependsOn"], json!(["countryGeoId"]));
    assert_eq!(state["w"]["optionsParameters"], json!(["countryGeoId"]));
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
