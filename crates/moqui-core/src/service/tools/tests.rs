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

//! The tools services on the in-memory database: print jobs and the services that are not available yet. The clean
//! services compare stamps, which the in-memory database does not; they and the print jobs on Postgres, with the
//! screens, are `moqui-host/tests/tools_services.rs`.

use std::collections::BTreeMap;

use moqui_lib::object::{to_plain_string, Value};

use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;
use crate::service::ServicePools;

const PRINT_JOB: &str = "moqui.basic.print.PrintJob";

fn setup() -> (EntityFacade, ExecutionContext) {
    let facade = crate::testing::open();
    let context =
        ExecutionContextFactory::test_context_in("", ServicePools::bounded(1, 4), crate::context::framework_dir());
    context.factory().set_entity(facade.clone());
    (facade, context)
}

fn install(files: &[&str]) {
    let dir = crate::context::framework_dir().join("service/org/moqui/impl");
    let paths: Vec<_> = files.iter().map(|file| dir.join(file)).collect();
    let failures = crate::service::lint(&paths).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
}

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), Value::string(*value)))
        .collect()
}

fn errors(context: &ExecutionContext) -> String {
    context.message().errors().join("\n")
}

fn job_field(facade: &EntityFacade, id: &str, field: &str) -> String {
    facade
        .find(PRINT_JOB)
        .unwrap()
        .eq("printJobId", Value::string(id))
        .one()
        .unwrap()
        .and_then(|row| row.get(field).map(to_plain_string))
        .unwrap_or_default()
}

#[test]
fn a_print_job_is_created_and_its_status_moves_to_a_failed_send() {
    let (facade, context) = setup();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    install(&["PrintServices.xml"]);

    let mut call = params(&[
        ("networkPrinterId", "GATE_PRINTER"),
        ("contentType", "text/plain"),
        ("jobName", "Test job"),
    ]);
    call.insert("inputStream".to_string(), Value::Bytes(b"hello printer".to_vec()));
    let out = crate::service::sync("org.moqui.impl.PrintServices.print#Document", &call).unwrap();
    let id = to_plain_string(out.get("printJobId").unwrap());
    assert!(!id.is_empty(), "{out:?}");
    assert!(
        errors(&context).contains("Network printing support not installed (moqui-cups is not part of this build)"),
        "{}",
        errors(&context)
    );
    // The create and the failed send each ran in their own transaction, so the record stays with the moved status.
    assert_eq!(job_field(&facade, &id, "statusId"), "PtjSendFailed");
    assert_eq!(job_field(&facade, &id, "jobName"), "Test job");
    assert_eq!(
        job_field(&facade, &id, "errorMessage"),
        "Network printing support not installed (moqui-cups is not part of this build)"
    );
    let stored = facade
        .find(PRINT_JOB)
        .unwrap()
        .eq("printJobId", Value::string(&id))
        .one()
        .unwrap()
        .unwrap();
    assert!(matches!(stored.get("document"), Some(Value::Bytes(bytes)) if bytes.as_slice() == b"hello printer"));

    // No document at all is the Java message.
    context.message().clear();
    crate::service::sync(
        "org.moqui.impl.PrintServices.print#Document",
        &params(&[("networkPrinterId", "P")]),
    )
    .unwrap();
    assert!(
        errors(&context).contains("Both inputStream and serialBlob are null"),
        "{}",
        errors(&context)
    );
}

struct FakePdf;

impl crate::screen::PdfEngine for FakePdf {
    fn render(&self, html: &str) -> Result<Vec<u8>, crate::context::Error> {
        assert!(html.contains("@page"), "the screen renders as html-print: {html}");
        Ok(b"%PDF-1.7 fake".to_vec())
    }
}

#[test]
fn a_screen_document_prints_its_pdf_or_reports_that_pdf_is_not_available() {
    let _pdf = crate::screen::pdf_test_lock();
    crate::screen::clear_pdf_engine();
    let (facade, context) = setup();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    install(&["PrintServices.xml"]);
    let location = format!(
        "{}/src/screen/testdata/Render/Render/Print.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    let call = params(&[
        ("networkPrinterId", "P"),
        ("screenLocation", &location),
        ("jobName", "Screen job"),
    ]);
    crate::service::sync("org.moqui.impl.PrintServices.print#ScreenDocument", &call).unwrap();
    assert_eq!(errors(&context), crate::screen::PDF_UNAVAILABLE);

    // With an engine the PDF is stored on the job, and the send fails as any other.
    context.message().clear();
    crate::screen::install_pdf_engine(std::sync::Arc::new(FakePdf));
    let out = crate::service::sync("org.moqui.impl.PrintServices.print#ScreenDocument", &call).unwrap();
    crate::screen::clear_pdf_engine();
    let id = to_plain_string(out.get("printJobId").unwrap());
    assert!(!id.is_empty(), "{out:?} {}", errors(&context));
    assert_eq!(job_field(&facade, &id, "contentType"), "application/pdf");
    assert_eq!(job_field(&facade, &id, "statusId"), "PtjSendFailed");
    let stored = facade
        .find(PRINT_JOB)
        .unwrap()
        .eq("printJobId", Value::string(&id))
        .one()
        .unwrap()
        .unwrap();
    assert!(matches!(stored.get("document"), Some(Value::Bytes(bytes)) if bytes.starts_with(b"%PDF-")));

    context.message().clear();
    crate::service::sync(
        "org.moqui.impl.PrintServices.print#ScreenDocument",
        &params(&[
            ("networkPrinterId", "P"),
            ("screenLocation", "file:/no/such/NotThere.xml"),
        ]),
    )
    .unwrap();
    assert!(
        errors(&context).contains("Screen not found at [file:/no/such/NotThere.xml]"),
        "{}",
        errors(&context)
    );
}

#[test]
fn llm_a2a_instance_wiki_and_recaptcha_services_name_why_they_do_not_run() {
    let (_facade, context) = setup();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    install(&[
        "LlmServices.xml",
        "A2AServices.xml",
        "InstanceServices.xml",
        // get#WikiPageManualDocumentData implements add#ManualDocumentData.
        "../EntityServices.xml",
        "WikiServices.xml",
        "GoogleServices.xml",
    ]);
    let err = crate::service::sync("org.moqui.impl.LlmServices.clean#LlmData", &BTreeMap::new()).unwrap_err();
    assert!(
        err.message().starts_with(
            "service org.moqui.impl.LlmServices.clean#LlmData is not available: LLM is not available until phase 13"
        ),
        "{err}"
    );
    // A failed call leaves its error in the messages, and a call after an error does not run.
    context.message().clear();
    let err = crate::service::sync("org.moqui.impl.A2AServices.clean#A2AData", &BTreeMap::new()).unwrap_err();
    assert!(err.message().contains("A2A is not available until phase 13"), "{err}");
    context.message().clear();
    let err = crate::service::sync(
        "org.moqui.impl.InstanceServices.get#DockerStatus",
        &params(&[("appInstanceId", "X")]),
    )
    .unwrap_err();
    assert!(
        err.message().contains("Docker app instance provisioning is not ported"),
        "{err}"
    );
    context.message().clear();
    let err = crate::service::sync(
        "org.moqui.impl.WikiServices.get#WikiPageInfo",
        &params(&[("wikiSpaceId", "X"), ("pagePath", "a")]),
    )
    .unwrap_err();
    assert!(
        err.message().starts_with(
            "service org.moqui.impl.WikiServices.get#WikiPageInfo is not available: the wiki services are not ported"
        ),
        "{err}"
    );
    context.message().clear();
    let err = crate::service::sync(
        "org.moqui.impl.GoogleServices.verify#ReCaptcha",
        &params(&[("reCaptchaToken", "t"), ("reCaptchaSecret", "s")]),
    )
    .unwrap_err();
    assert!(err.message().contains("reCAPTCHA verification is not ported"), "{err}");
}
