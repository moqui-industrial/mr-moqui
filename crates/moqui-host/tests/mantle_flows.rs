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

//! Phase 0b: the Mantle USL Spock flows (`runtime/component/mantle-usl/src/test/groovy`, the oracle) driven from
//! Rust. One embedded database is loaded once per test binary with every data type of `mantle-udm` and
//! `mantle-usl` (`mantle_support`), then each flow makes the same service calls and entity assertions as its
//! Groovy file. See `plans/MantleFlowTests.md` for how to run them and the status of each.
//!
//! ```text
//! cargo test -p moqui-host --test mantle_flows                    # flows that pass today
//! cargo test -p moqui-host --test mantle_flows -- --ignored       # flows that cannot pass yet
//! cargo test -p moqui-host --test mantle_flows -- --include-ignored --test-threads=1
//! ```
//!
//! Differences from the Spock files, all in the harness: no `tempSetSequencedIdPrimary` and no
//! `setEffectiveTime`, so generated ids come from service results and checks leave dates out; the Spock
//! `makeDataLoader().xmlText(..).check()` is `mantle_support::check`, which skips relationship elements.
//!
//! # Mantle services each flow calls
//!
//! `FLOW_SERVICES` below lists them per flow (the same names as the `.name("...")` calls in the Groovy file, minus
//! entity-auto `create#`/`update#` calls); `service_readiness` reports which of them register. In short:
//!
//! | Flow (Groovy file) | Services called | Calls |
//! |---|---|---|
//! | OrderToCashBasicFlow | PriceServices, OrderServices (add/update/create#OrderItem/set#OrderBillingShippingInfo/place/approve/cancel/create/add#OrderPartPayment), ShipmentServices (create#OrderPartShipment, pack*, ship, cancel, unpack), InvoiceServices, PaymentServices, AssetServices.reserve | 25 |
//! | OrderProcureToPayBasicFlow | PriceServices, OrderServices, ShipmentServices (receive, deliver, ship), AssetServices (move, record#PhysicalInventoryChange), InvoiceServices (adjust, get#InvoiceTotal), PaymentServices, NachaServices, LedgerServices, AssetAutoPostServices | 23 |
//! | ReturnToResponseBasicFlow | ReturnServices (create/add/request/approve), ShipmentServices (receive, deliver, ship#OrderPart), OrderServices | 11 |
//! | WorkPlanToCashBasicFlow | PartyServices, ContactServices, LedgerServices.init#PartyAccountingConfiguration, ProjectServices, TaskServices, RequestServices, InvoiceServices (project invoices), PaymentServices.create#InvoicePayment | 15 |
//! | AccountingActivities | LedgerServices (create#AcctgTrans, create#AcctgTransEntry, post, unpost, get#OrganizationFiscalTimePeriods), InvoiceServices | 7 |
//! | OrderToCashTime | OrderServices, ShipmentServices.ship#OrderPart | 5 |
//! | AssetReservationMultipleThreads | OrderServices (add, set#OrderBillingShippingInfo, place, cancel) | 4 |
//! | OrderTenantAccess | OrderServices (add, set#OrderBillingShippingInfo, place) | 3 |
//! | RestApiTests | LedgerServices.recalculate#GlAccountOrgSummaries, then REST screens `s1/mantle/...` | 1 |
//! | MantleUslSuite | runs the flows above in the suite's order (OrderTenantAccess is left out of the suite) | |
//!
//! Every flow test is `#[ignore]` with the first failing service as its reason until it passes; remove the
//! attribute when it does.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres"))]

mod mantle_support;

use mantle_support::*;
use moqui_lib::object::Value;

/// The services each flow calls by name (entity-auto `create#`/`update#` calls are implicit and left out), in the
/// order of the first call. `service_readiness` checks every one against the registry.
const FLOW_SERVICES: &[(&str, &[&str])] = &[
    (
        "OrderToCashBasicFlow",
        &[
            "mantle.product.PriceServices.get#ProductPrice",
            "mantle.order.OrderServices.add#OrderProductQuantity",
            "mantle.order.OrderServices.update#OrderPart",
            "mantle.order.OrderServices.create#OrderItem",
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            "mantle.order.OrderServices.place#Order",
            "mantle.order.OrderServices.approve#Order",
            "mantle.shipment.ShipmentServices.ship#OrderPart",
            "mantle.shipment.ShipmentServices.create#OrderPartShipment",
            "mantle.shipment.ShipmentServices.pack#ShipmentProduct",
            "mantle.shipment.ShipmentServices.pack#ShipmentAssemblyComponent",
            "mantle.shipment.ShipmentServices.pack#Shipment",
            "mantle.shipment.ShipmentServices.ship#Shipment",
            "mantle.order.OrderServices.cancel#Order",
            "mantle.order.OrderServices.create#Order",
            "mantle.order.OrderServices.add#OrderPartPayment",
            "mantle.account.InvoiceServices.create#Invoice",
            "mantle.account.InvoiceServices.create#InvoiceItem",
            "mantle.account.PaymentServices.apply#InvoiceToInvoice",
            "mantle.account.PaymentServices.update#Payment",
            "mantle.account.PaymentServices.create#Payment",
            "mantle.account.PaymentServices.apply#PaymentToPayment",
            "mantle.shipment.ShipmentServices.cancel#Shipment",
            "mantle.shipment.ShipmentServices.unpack#ShipmentItemIssuance",
            "mantle.product.AssetServices.reserve#AssetForOrderItem",
        ],
    ),
    (
        "OrderProcureToPayBasicFlow",
        &[
            "mantle.product.PriceServices.get#ProductPrice",
            "mantle.order.OrderServices.create#Order",
            "mantle.order.OrderServices.add#OrderProductQuantity",
            "mantle.order.OrderServices.create#OrderItem",
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            "mantle.order.OrderServices.place#Order",
            "mantle.order.OrderServices.approve#Order",
            "mantle.shipment.ShipmentServices.create#OrderPartShipment",
            "mantle.shipment.ShipmentServices.ship#Shipment",
            "mantle.shipment.ShipmentServices.receive#ShipmentProduct",
            "mantle.shipment.ShipmentServices.deliver#Shipment",
            "mantle.product.AssetServices.move#Product",
            "mantle.account.InvoiceServices.adjust#Invoice",
            "mantle.account.NachaServices.generate#NachaFile",
            "mantle.account.PaymentServices.create#Payment",
            "mantle.account.PaymentServices.apply#PaymentToPayment",
            "mantle.ledger.LedgerServices.get#OrganizationFiscalTimePeriods",
            "mantle.ledger.AssetAutoPostServices.calculateAndPost#AllFixedAssetDepreciations",
            "mantle.shipment.ShipmentServices.pack#ShipmentProduct",
            "mantle.shipment.ShipmentServices.pack#Shipment",
            "mantle.account.InvoiceServices.get#InvoiceTotal",
            "mantle.account.PaymentServices.create#InvoicePayment",
            "mantle.product.AssetServices.record#PhysicalInventoryChange",
        ],
    ),
    (
        "ReturnToResponseBasicFlow",
        &[
            "mantle.order.ReturnServices.create#ReturnFromOrder",
            "mantle.order.ReturnServices.add#OrderItemToReturn",
            "mantle.order.ReturnServices.request#Return",
            "mantle.order.ReturnServices.approve#Return",
            "mantle.shipment.ShipmentServices.receive#ShipmentProduct",
            "mantle.shipment.ShipmentServices.deliver#Shipment",
            "mantle.shipment.ShipmentServices.ship#OrderPart",
            "mantle.order.OrderServices.add#OrderProductQuantity",
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            "mantle.order.OrderServices.place#Order",
            "mantle.order.OrderServices.approve#Order",
        ],
    ),
    (
        "WorkPlanToCashBasicFlow",
        &[
            "mantle.party.PartyServices.create#Organization",
            "mantle.party.ContactServices.store#PartyContactInfo",
            "mantle.ledger.LedgerServices.init#PartyAccountingConfiguration",
            "mantle.party.PartyServices.create#Account",
            "mantle.work.ProjectServices.create#Project",
            "mantle.work.ProjectServices.update#Project",
            "mantle.work.ProjectServices.create#Milestone",
            "mantle.work.TaskServices.create#Task",
            "mantle.work.TaskServices.update#Task",
            "mantle.work.TaskServices.add#TaskTime",
            "mantle.request.RequestServices.create#Request",
            "mantle.request.RequestServices.update#Request",
            "mantle.account.InvoiceServices.create#ProjectExpenseInvoice",
            "mantle.account.InvoiceServices.create#ProjectInvoiceItems",
            "mantle.account.PaymentServices.create#InvoicePayment",
        ],
    ),
    (
        "AccountingActivities",
        &[
            "mantle.ledger.LedgerServices.get#OrganizationFiscalTimePeriods",
            "mantle.ledger.LedgerServices.create#AcctgTrans",
            "mantle.ledger.LedgerServices.create#AcctgTransEntry",
            "mantle.ledger.LedgerServices.post#AcctgTrans",
            "mantle.ledger.LedgerServices.unpost#AcctgTrans",
            "mantle.account.InvoiceServices.create#Invoice",
            "mantle.account.InvoiceServices.create#InvoiceItem",
        ],
    ),
    (
        "OrderToCashTime",
        &[
            "mantle.order.OrderServices.add#OrderProductQuantity",
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            "mantle.order.OrderServices.place#Order",
            "mantle.order.OrderServices.approve#Order",
            "mantle.shipment.ShipmentServices.ship#OrderPart",
        ],
    ),
    (
        "AssetReservationMultipleThreads",
        &[
            "mantle.order.OrderServices.add#OrderProductQuantity",
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            "mantle.order.OrderServices.place#Order",
            "mantle.order.OrderServices.cancel#Order",
        ],
    ),
    (
        "OrderTenantAccess",
        &[
            "mantle.order.OrderServices.add#OrderProductQuantity",
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            "mantle.order.OrderServices.place#Order",
        ],
    ),
    (
        "RestApiTests",
        &["mantle.ledger.LedgerServices.recalculate#GlAccountOrgSummaries"],
    ),
];

/// Which of each flow's services register today. It runs inside the fixture because service files resolve
/// `component://` locations through the booted runtime. Run with `-- --nocapture` for the table that goes into
/// `plans/MantleFlowTests.md`; it fails only when a service a flow calls is not defined at all.
#[test]
fn service_readiness() {
    with_flow("service_readiness", |_f| {
        let mut undefined = Vec::new();
        for (flow, services) in FLOW_SERVICES {
            let mut groovy = Vec::new();
            for service in *services {
                match service_status(service) {
                    Ok(()) => {}
                    Err(why) if why.contains("is Groovy") || why.contains("is not JavaScript") => groovy.push(*service),
                    Err(why) if why.starts_with("service ") => {
                        eprintln!("READINESS {flow}: {service} fails to register: {why}");
                        groovy.push(*service);
                    }
                    Err(why) => undefined.push(format!("{flow}: {service}: {why}")),
                }
            }
            eprintln!(
                "READINESS {flow}: {} of {} service(s) register; Groovy: {groovy:?}",
                services.len() - groovy.len(),
                services.len()
            );
        }
        assert!(
            undefined.is_empty(),
            "services a flow calls that do not exist: {undefined:#?}"
        );
    });
}

// ---------------------------------------------------------------------------------------------------------------
// OrderToCashBasicFlow
// ---------------------------------------------------------------------------------------------------------------

/// The order, shipment, invoice and payment ids the later features of OrderToCashBasicFlow read (the Spock
/// `@Shared` fields).
#[derive(Default)]
struct O2c {
    order_id: String,
    order_part_seq_id: String,
    payment_id: String,
    shipment_id: String,
    b2b_order_id: String,
    b2b_payment_id: String,
    b2b_invoice_id: String,
    b2b_cred_memo_id: String,
    inventory_order_id: String,
}

fn assert_clean(errors: Vec<String>, step: &str) {
    assert!(errors.is_empty(), "{step}: data check: {errors:#?}");
}

fn o2c_create_sales_order(f: &Flow, st: &mut O2c) {
    login("joe@public.com");
    let store = one(
        f,
        "mantle.product.store.ProductStore",
        &[("productStoreId", "POPC_DEFAULT")],
    )
    .expect("store");
    let currency = text(&store, "defaultCurrencyUomId");
    let vendor = text(&store, "organizationPartyId");
    assert_eq!(vendor, "ORG_ZIZI_RETAIL");
    let customer = "CustJqp";

    let price = call(
        "mantle.product.PriceServices.get#ProductPrice",
        &[
            ("productId", s("DEMO_1_1")),
            ("priceUomId", s(&currency)),
            ("productStoreId", s("POPC_DEFAULT")),
            ("vendorPartyId", s(&vendor)),
            ("customerPartyId", s(customer)),
        ],
    );
    assert_eq!(number(price.get("price")), 16.99);
    assert_eq!(text(&price, "priceUomId"), "USD");

    let add1 = call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &[
            ("productId", s("DEMO_1_1")),
            ("quantity", n("1")),
            ("customerPartyId", s(customer)),
            ("currencyUomId", s(&currency)),
            ("productStoreId", s("POPC_DEFAULT")),
        ],
    );
    st.order_id = need(&add1, "orderId");
    st.order_part_seq_id = need(&add1, "orderPartSeqId");
    let (oid, part) = (st.order_id.clone(), st.order_part_seq_id.clone());

    call(
        "mantle.order.OrderServices.update#OrderPart",
        &[
            ("orderId", s(&oid)),
            ("orderPartSeqId", s(&part)),
            ("disablePromotions", s("Y")),
            ("disableShippingCalc", s("Y")),
            ("disableTaxCalc", s("Y")),
        ],
    );
    let add2 = call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &[("orderId", s(&oid)), ("productId", s("DEMO_3_1")), ("quantity", n("5"))],
    );
    let add3 = call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &[
            ("orderId", s(&oid)),
            ("orderPartSeqId", s(&part)),
            ("productId", s("DEMO_2_1")),
            ("quantity", n("7")),
            ("requireInventory", Value::Bool(false)),
        ],
    );
    let item = |parent: Option<&str>, kind: &str, desc: &str, qty: &str, amount: &str| {
        let mut params = vec![
            ("orderId", s(&oid)),
            ("orderPartSeqId", s(&part)),
            ("itemTypeEnumId", s(kind)),
            ("itemDescription", s(desc)),
            ("quantity", n(qty)),
            ("unitAmount", n(amount)),
        ];
        if let Some(parent) = parent {
            params.push(("parentItemSeqId", s(parent)));
        }
        call("mantle.order.OrderServices.create#OrderItem", &params);
    };
    let item1 = need(&add1, "orderItemSeqId");
    let item2 = need(&add2, "orderItemSeqId");
    let item3 = need(&add3, "orderItemSeqId");
    item(Some(&item1), "ItemSalesTax", "Test Tax 7%", "1", "1.189");
    item(
        Some(&item2),
        "ItemDiscount",
        "Discount why? Because we love you.",
        "5",
        "-1.23",
    );
    item(Some(&item2), "ItemSalesTax", "Test Tax 7%", "5", "0.458");
    item(
        Some(&item3),
        "ItemDiscount",
        "Discount why? Because we love you.",
        "4",
        "-2.345",
    );
    item(Some(&item3), "ItemSalesTax", "Test Tax 7%", "1", "5.28");

    // a pick assembly product
    call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &[
            ("orderId", s(&oid)),
            ("orderPartSeqId", s(&part)),
            ("productId", s("DEMO_PA")),
            ("quantity", n("2")),
            ("unitAmount", n("30.0")),
            ("requireInventory", Value::Bool(false)),
        ],
    );
    item(None, "ItemShipping", "Standard Shipping", "1", "6.77");

    let info = call(
        "mantle.order.OrderServices.set#OrderBillingShippingInfo",
        &[
            ("orderId", s(&oid)),
            ("paymentMethodId", s("CustJqpCc")),
            ("shippingPostalContactMechId", s("CustJqpAddr")),
            ("shippingTelecomContactMechId", s("CustJqpTeln")),
            ("carrierPartyId", s("_NA_")),
            ("shipmentMethodEnumId", s("ShMthGround")),
        ],
    );
    st.payment_id = need(&info, "paymentId");
    call(
        "mantle.order.OrderServices.place#Order",
        &[("orderId", s(&oid)), ("requireInventory", Value::Bool(false))],
    );
    logout();
    login("john.doe");
    call("mantle.order.OrderServices.approve#Order", &[("orderId", s(&oid))]);
    logout();

    let xml = format!(
        r#"<entity-facade-xml>
        <mantle.order.OrderHeader orderId="{oid}" statusId="OrderApproved" currencyUomId="USD" productStoreId="POPC_DEFAULT" grandTotal="200.68"/>
        <mantle.account.payment.Payment paymentId="{pay}" paymentTypeEnumId="PtInvoicePayment" paymentMethodId="CustJqpCc"
            paymentInstrumentEnumId="PiCreditCard" orderId="{oid}" orderPartSeqId="01" statusId="PmntAuthorized" amount="200.68"
            amountUomId="USD" fromPartyId="CustJqp" toPartyId="ORG_ZIZI_RETAIL"/>
        <mantle.order.OrderPart orderId="{oid}" orderPartSeqId="01" vendorPartyId="ORG_ZIZI_RETAIL" customerPartyId="CustJqp"
            shipmentMethodEnumId="ShMthGround" postalContactMechId="CustJqpAddr" telecomContactMechId="CustJqpTeln" partTotal="200.68"/>
        <mantle.order.OrderItem orderId="{oid}" orderItemSeqId="01" orderPartSeqId="01" itemTypeEnumId="ItemProduct"
            productId="DEMO_1_1" itemDescription="Demo Product One-One" quantity="1" unitAmount="16.99" unitListPrice="19.99" isModifiedPrice="N"/>
        <mantle.order.OrderItem orderId="{oid}" orderItemSeqId="02" orderPartSeqId="01" itemTypeEnumId="ItemProduct"
            productId="DEMO_3_1" itemDescription="Demo Product Three-One" quantity="5" unitAmount="7.77" isModifiedPrice="N"/>
        <mantle.order.OrderItem orderId="{oid}" orderItemSeqId="03" orderPartSeqId="01" itemTypeEnumId="ItemProduct"
            productId="DEMO_2_1" itemDescription="Demo Product Two-One" quantity="7" unitAmount="12.12" isModifiedPrice="N"/>
    </entity-facade-xml>"#,
        pay = st.payment_id
    );
    assert_clean(check(f, &xml), "create Sales Order");
    // PaymentGatewayResponse for the authorization (id is sequenced, so found by payment)
    let responses = list(
        f,
        "mantle.account.method.PaymentGatewayResponse",
        &[("paymentId", &st.payment_id)],
    );
    assert!(
        responses
            .iter()
            .any(|r| text(r, "paymentOperationEnumId") == "PgoAuthorize" && text(r, "resultSuccess") == "Y"),
        "authorize response: {responses:?}"
    );
}

fn o2c_validate_asset_reservation(f: &Flow, st: &mut O2c) {
    let oid = &st.order_id;
    // reservations by order item; asset ids of the generated Asset (55500) are sequenced, so look it up
    let res = |item: &str| {
        list(
            f,
            "mantle.product.issuance.AssetReservation",
            &[("orderId", oid), ("orderItemSeqId", item)],
        )
    };
    let r1 = res("01");
    assert_eq!(r1.len(), 1, "{r1:?}");
    assert_eq!(text(&r1[0], "assetId"), "55400");
    assert_eq!(number(r1[0].get("quantity")), 1.0);
    let a = one(f, "mantle.product.asset.Asset", &[("assetId", "55400")]).unwrap();
    assert_eq!(number(a.get("quantityOnHandTotal")), 400.0);
    assert_eq!(number(a.get("availableToPromiseTotal")), 187.0);
    let r2 = res("03");
    assert_eq!(r2.len(), 1, "{r2:?}");
    assert_eq!(number(r2[0].get("quantityNotAvailable")), 7.0);
    let auto = one(
        f,
        "mantle.product.asset.Asset",
        &[("assetId", &text(&r2[0], "assetId"))],
    )
    .unwrap();
    assert_eq!(text(&auto, "productId"), "DEMO_2_1");
    assert_eq!(number(auto.get("availableToPromiseTotal")), -11.0);
    let r3 = res("02");
    assert_eq!(r3.len(), 1, "{r3:?}");
    assert_eq!(text(&r3[0], "assetId"), "DEMO_3_1A");
    let a3 = one(f, "mantle.product.asset.Asset", &[("assetId", "DEMO_3_1A")]).unwrap();
    assert_eq!(number(a3.get("availableToPromiseTotal")), 0.0);
}

fn o2c_ship_sales_order(f: &Flow, st: &mut O2c) {
    login("john.doe");
    let ship = call(
        "mantle.shipment.ShipmentServices.create#OrderPartShipment",
        &[
            ("orderId", s(&st.order_id)),
            ("orderPartSeqId", s(&st.order_part_seq_id)),
            ("createPackage", Value::Bool(true)),
        ],
    );
    st.shipment_id = need(&ship, "shipmentId");
    let package = need(&ship, "shipmentPackageSeqId");
    let sid = st.shipment_id.clone();
    let pack = |product: &str, qty: &str| {
        call(
            "mantle.shipment.ShipmentServices.pack#ShipmentProduct",
            &[
                ("productId", s(product)),
                ("quantity", n(qty)),
                ("shipmentId", s(&sid)),
                ("shipmentPackageSeqId", s(&package)),
            ],
        );
    };
    pack("DEMO_1_1", "1");
    pack("DEMO_3_1", "4"); // partial fill, 4 of 5
    pack("DEMO_2_1", "5"); // partial fill, 5 of 7
    let component = |product: &str, qty: &str| {
        call(
            "mantle.shipment.ShipmentServices.pack#ShipmentAssemblyComponent",
            &[
                ("productId", s(product)),
                ("assemblyProductId", s("DEMO_PA")),
                ("quantity", n(qty)),
                ("shipmentId", s(&sid)),
                ("shipmentPackageSeqId", s(&package)),
            ],
        );
    };
    component("DEMO_1_1", "1");
    component("DEMO_1_1", "1");
    component("DEMO_2_1", "2");
    component("DEMO_2_1", "2");
    call(
        "mantle.shipment.ShipmentServices.pack#Shipment",
        &[("shipmentId", s(&sid))],
    );
    call(
        "mantle.shipment.ShipmentServices.ship#Shipment",
        &[("shipmentId", s(&sid))],
    );

    let xml = format!(
        r#"<entity-facade-xml>
        <mantle.shipment.Shipment shipmentId="{sid}" shipmentTypeEnumId="ShpTpSales" statusId="ShipShipped" fromPartyId="ORG_ZIZI_RETAIL" toPartyId="CustJqp"/>
        <mantle.shipment.ShipmentPackage shipmentId="{sid}" shipmentPackageSeqId="01"/>
        <mantle.shipment.ShipmentItem shipmentId="{sid}" productId="DEMO_1_1" quantity="1"/>
        <mantle.shipment.ShipmentPackageContent shipmentId="{sid}" shipmentPackageSeqId="01" productId="DEMO_1_1" quantity="1"/>
        <mantle.shipment.ShipmentItem shipmentId="{sid}" productId="DEMO_3_1" quantity="4"/>
        <mantle.shipment.ShipmentPackageContent shipmentId="{sid}" shipmentPackageSeqId="01" productId="DEMO_3_1" quantity="4"/>
        <mantle.shipment.ShipmentItem shipmentId="{sid}" productId="DEMO_2_1" quantity="5"/>
        <mantle.shipment.ShipmentPackageContent shipmentId="{sid}" shipmentPackageSeqId="01" productId="DEMO_2_1" quantity="5"/>
        <mantle.shipment.ShipmentRouteSegment shipmentId="{sid}" shipmentRouteSegmentSeqId="01"
            destPostalContactMechId="CustJqpAddr" destTelecomContactMechId="CustJqpTeln"/>
        <mantle.shipment.ShipmentPackageRouteSeg shipmentId="{sid}" shipmentPackageSeqId="01" shipmentRouteSegmentSeqId="01"/>
    </entity-facade-xml>"#
    );
    assert_clean(check(f, &xml), "ship Sales Order");
    let sources = list(f, "mantle.shipment.ShipmentItemSource", &[("shipmentId", &sid)]);
    assert!(
        sources.iter().all(|r| text(r, "statusId") == "SisPacked"),
        "{sources:?}"
    );
    assert!(
        sources.iter().all(|r| !text(r, "invoiceId").is_empty()),
        "the SECA invoiced the shipment: {sources:?}"
    );
}

fn o2c_close_partial_filled_order(f: &Flow, st: &mut O2c) {
    call(
        "mantle.order.OrderServices.cancel#Order",
        &[("orderId", s(&st.order_id))],
    );
    let xml = format!(
        r#"<entity-facade-xml><mantle.order.OrderHeader orderId="{}" statusId="OrderCompleted" currencyUomId="USD"
            productStoreId="POPC_DEFAULT" grandTotal="170.61"/></entity-facade-xml>"#,
        st.order_id
    );
    assert_clean(check(f, &xml), "close Partial Filled Order");
}

fn o2c_validate_asset_issuance(f: &Flow, st: &mut O2c) {
    let a = one(f, "mantle.product.asset.Asset", &[("assetId", "55400")]).unwrap();
    assert_eq!(number(a.get("quantityOnHandTotal")), 397.0);
    assert_eq!(number(a.get("availableToPromiseTotal")), 187.0);
    let a3 = one(f, "mantle.product.asset.Asset", &[("assetId", "DEMO_3_1A")]).unwrap();
    assert_eq!(number(a3.get("quantityOnHandTotal")), 1.0);
    assert_eq!(number(a3.get("availableToPromiseTotal")), 1.0);
    let issued = list(
        f,
        "mantle.product.issuance.AssetIssuance",
        &[("shipmentId", &st.shipment_id)],
    );
    let qty = |product: &str| -> f64 {
        issued
            .iter()
            .filter(|r| text(r, "productId") == product)
            .map(|r| number(r.get("quantity")))
            .sum()
    };
    assert_eq!(qty("DEMO_1_1"), 1.0, "{issued:?}");
    assert_eq!(qty("DEMO_3_1"), 4.0);
    assert_eq!(qty("DEMO_2_1"), 5.0);
}

fn o2c_validate_issuance_accounting(f: &Flow, st: &mut O2c) {
    let issued = list(
        f,
        "mantle.product.issuance.AssetIssuance",
        &[("shipmentId", &st.shipment_id)],
    );
    assert!(!issued.is_empty());
    for issuance in &issued {
        let trans = list(
            f,
            "mantle.ledger.transaction.AcctgTrans",
            &[("assetIssuanceId", &text(issuance, "assetIssuanceId"))],
        );
        assert_eq!(trans.len(), 1, "one AttInventoryIssuance per issuance: {issuance:?}");
        assert_eq!(text(&trans[0], "acctgTransTypeEnumId"), "AttInventoryIssuance");
        assert_eq!(text(&trans[0], "isPosted"), "Y");
        let entries = list(
            f,
            "mantle.ledger.transaction.AcctgTransEntry",
            &[("acctgTransId", &text(&trans[0], "acctgTransId"))],
        );
        assert_eq!(entries.len(), 2, "{entries:?}");
    }
}

fn o2c_validate_shipment_invoice(f: &Flow, st: &mut O2c) {
    let sources = list(
        f,
        "mantle.shipment.ShipmentItemSource",
        &[("shipmentId", &st.shipment_id)],
    );
    let invoice_id = text(&sources[0], "invoiceId");
    let inv = one(f, "mantle.account.invoice.Invoice", &[("invoiceId", &invoice_id)]).expect("invoice");
    assert_eq!(text(&inv, "invoiceTypeEnumId"), "InvoiceSales");
    assert_eq!(text(&inv, "toPartyId"), "CustJqp");
    assert_eq!(text(&inv, "fromPartyId"), "ORG_ZIZI_RETAIL");
    assert_eq!(number(inv.get("invoiceTotal")), 170.61);
    assert_eq!(text(&inv, "statusId"), "InvoicePmtRecvd");
    assert_eq!(number(inv.get("unpaidTotal")), 0.0);
    assert_eq!(number(inv.get("appliedPaymentsTotal")), 170.61);
    let apps = list(
        f,
        "mantle.account.payment.PaymentApplication",
        &[("invoiceId", &invoice_id)],
    );
    assert_eq!(apps.len(), 1, "{apps:?}");
    assert_eq!(number(apps[0].get("amountApplied")), 170.61);
    assert_eq!(text(&apps[0], "paymentId"), st.payment_id);
}

fn o2c_validate_invoice_and_payment_accounting(f: &Flow, st: &mut O2c) {
    let sources = list(
        f,
        "mantle.shipment.ShipmentItemSource",
        &[("shipmentId", &st.shipment_id)],
    );
    let invoice_id = text(&sources[0], "invoiceId");
    let inv_trans = list(f, "mantle.ledger.transaction.AcctgTrans", &[("invoiceId", &invoice_id)]);
    assert!(!inv_trans.is_empty(), "the finalized invoice posted to the GL");
    assert_eq!(text(&inv_trans[0], "isPosted"), "Y");
    let pay_trans = list(
        f,
        "mantle.ledger.transaction.AcctgTrans",
        &[("paymentId", &st.payment_id)],
    );
    assert!(!pay_trans.is_empty(), "the payment posted to the GL");
    let pay = one(f, "mantle.account.payment.Payment", &[("paymentId", &st.payment_id)]).unwrap();
    assert_eq!(text(&pay, "statusId"), "PmntDelivered");
}

fn o2c_create_and_ship_business_customer_order(f: &Flow, st: &mut O2c) {
    login("john.doe");
    let created = call(
        "mantle.order.OrderServices.create#Order",
        &[
            ("vendorPartyId", s("ORG_ZIZI_RETAIL")),
            ("customerPartyId", s("JoeDist")),
            ("facilityId", s("ZIRET_WH")),
        ],
    );
    st.b2b_order_id = need(&created, "orderId");
    let part = need(&created, "orderPartSeqId");
    let oid = st.b2b_order_id.clone();
    for (product, qty, amount) in [("DEMO_1_1", "100.0", "15.0"), ("DEMO_3_1", "20.0", "5.5")] {
        call(
            "mantle.order.OrderServices.add#OrderProductQuantity",
            &[
                ("orderId", s(&oid)),
                ("orderPartSeqId", s(&part)),
                ("productId", s(product)),
                ("quantity", n(qty)),
                ("unitAmount", n(amount)),
            ],
        );
    }
    call(
        "mantle.order.OrderServices.set#OrderBillingShippingInfo",
        &[
            ("orderId", s(&oid)),
            ("orderPartSeqId", s(&part)),
            ("shippingPostalContactMechId", s("JoeDistAddr")),
            ("carrierPartyId", s("_NA_")),
            ("shipmentMethodEnumId", s("ShMthGround")),
        ],
    );
    let pay = call(
        "mantle.order.OrderServices.add#OrderPartPayment",
        &[
            ("orderId", s(&oid)),
            ("orderPartSeqId", s(&part)),
            ("paymentInstrumentEnumId", s("PiCompanyCheck")),
        ],
    );
    st.b2b_payment_id = need(&pay, "paymentId");
    call("mantle.order.OrderServices.place#Order", &[("orderId", s(&oid))]);
    call("mantle.order.OrderServices.approve#Order", &[("orderId", s(&oid))]);
    let ship = call(
        "mantle.shipment.ShipmentServices.create#OrderPartShipment",
        &[
            ("orderId", s(&oid)),
            ("orderPartSeqId", s(&part)),
            ("createPackage", Value::Bool(true)),
        ],
    );
    let sid = need(&ship, "shipmentId");
    let package = need(&ship, "shipmentPackageSeqId");
    for (product, qty) in [("DEMO_1_1", "100"), ("DEMO_3_1", "20")] {
        call(
            "mantle.shipment.ShipmentServices.pack#ShipmentProduct",
            &[
                ("productId", s(product)),
                ("quantity", n(qty)),
                ("shipmentId", s(&sid)),
                ("shipmentPackageSeqId", s(&package)),
            ],
        );
    }
    call(
        "mantle.shipment.ShipmentServices.pack#Shipment",
        &[("shipmentId", s(&sid))],
    );
    call(
        "mantle.shipment.ShipmentServices.ship#Shipment",
        &[("shipmentId", s(&sid))],
    );
    let sources = list(f, "mantle.shipment.ShipmentItemSource", &[("shipmentId", &sid)]);
    st.b2b_invoice_id = text(&sources[0], "invoiceId");
    let inv = one(
        f,
        "mantle.account.invoice.Invoice",
        &[("invoiceId", &st.b2b_invoice_id)],
    )
    .expect("b2b invoice");
    assert_eq!(number(inv.get("invoiceTotal")), 1610.0);
    let p = one(
        f,
        "mantle.account.payment.Payment",
        &[("paymentId", &st.b2b_payment_id)],
    )
    .unwrap();
    assert_eq!(text(&p, "paymentInstrumentEnumId"), "PiCompanyCheck");
}

fn o2c_credit_memo(f: &Flow, st: &mut O2c) {
    let memo = call(
        "mantle.account.InvoiceServices.create#Invoice",
        &[
            ("fromPartyId", s("JoeDist")),
            ("toPartyId", s("ORG_ZIZI_RETAIL")),
            ("invoiceTypeEnumId", s("InvoiceCreditMemo")),
        ],
    );
    st.b2b_cred_memo_id = need(&memo, "invoiceId");
    call(
        "mantle.account.InvoiceServices.create#InvoiceItem",
        &[
            ("invoiceId", s(&st.b2b_cred_memo_id)),
            ("itemTypeEnumId", s("ItemChargebackAdjust")),
            ("description", s("Test Chargeback")),
            ("quantity", n("1.0")),
            ("amount", n("250.0")),
        ],
    );
    call(
        "update#mantle.account.invoice.Invoice",
        &[
            ("invoiceId", s(&st.b2b_cred_memo_id)),
            ("statusId", s("InvoiceApproved")),
        ],
    );
    let inv = one(
        f,
        "mantle.account.invoice.Invoice",
        &[("invoiceId", &st.b2b_cred_memo_id)],
    )
    .unwrap();
    assert_eq!(text(&inv, "statusId"), "InvoiceApproved");
    assert_eq!(number(inv.get("invoiceTotal")), 250.0);
    let trans = list(
        f,
        "mantle.ledger.transaction.AcctgTrans",
        &[("invoiceId", &st.b2b_cred_memo_id)],
    );
    assert!(!trans.is_empty(), "the approved credit memo posted");
}

fn o2c_apply_credit_memo(f: &Flow, st: &mut O2c) {
    call(
        "mantle.account.PaymentServices.apply#InvoiceToInvoice",
        &[
            ("invoiceId", s(&st.b2b_cred_memo_id)),
            ("toInvoiceId", s(&st.b2b_invoice_id)),
        ],
    );
    let memo = one(
        f,
        "mantle.account.invoice.Invoice",
        &[("invoiceId", &st.b2b_cred_memo_id)],
    )
    .unwrap();
    assert_eq!(text(&memo, "statusId"), "InvoicePmtSent");
    assert_eq!(number(memo.get("unpaidTotal")), 0.0);
    let inv = one(
        f,
        "mantle.account.invoice.Invoice",
        &[("invoiceId", &st.b2b_invoice_id)],
    )
    .unwrap();
    assert_eq!(number(inv.get("appliedPaymentsTotal")), 250.0);
    assert_eq!(number(inv.get("unpaidTotal")), 1360.0);
}

fn o2c_receive_overpayment(f: &Flow, st: &mut O2c) {
    call(
        "mantle.account.PaymentServices.update#Payment",
        &[("paymentId", s(&st.b2b_payment_id)), ("amount", n("1500.0"))],
    );
    call(
        "mantle.account.PaymentServices.update#Payment",
        &[("paymentId", s(&st.b2b_payment_id)), ("statusId", s("PmntDelivered"))],
    );
    let p = one(
        f,
        "mantle.account.payment.Payment",
        &[("paymentId", &st.b2b_payment_id)],
    )
    .unwrap();
    assert_eq!(text(&p, "statusId"), "PmntDelivered");
    assert_eq!(number(p.get("amount")), 1500.0);
    assert_eq!(number(p.get("appliedTotal")), 1360.0);
    assert_eq!(number(p.get("unappliedTotal")), 140.0);
}

fn o2c_refund_overpayment(f: &Flow, st: &mut O2c) {
    let refund = call(
        "mantle.account.PaymentServices.create#Payment",
        &[
            ("paymentTypeEnumId", s("PtRefund")),
            ("statusId", s("PmntDelivered")),
            ("fromPartyId", s("ORG_ZIZI_RETAIL")),
            ("toPartyId", s("JoeDist")),
            ("effectiveDate", s(&utc_now_text())),
            ("paymentRefNum", s("1000")),
            ("paymentInstrumentEnumId", s("PiCompanyCheck")),
            ("paymentMethodId", s("ZIRET_BA")),
            ("amount", n("140.0")),
        ],
    );
    let applied = call(
        "mantle.account.PaymentServices.apply#PaymentToPayment",
        &[
            ("paymentId", s(&need(&refund, "paymentId"))),
            ("toPaymentId", s(&st.b2b_payment_id)),
        ],
    );
    assert_eq!(number(applied.get("amountApplied")), 140.0);
    let p = one(
        f,
        "mantle.account.payment.Payment",
        &[("paymentId", &st.b2b_payment_id)],
    )
    .unwrap();
    assert_eq!(number(p.get("unappliedTotal")), 0.0);
}

/// QOH minus ATP of an asset equals its reservations, the invariant the inventory features check.
fn reservation_invariant(f: &Flow, asset_id: &str) {
    let summary = one(f, "mantle.product.asset.AssetDetailSummary", &[("assetId", asset_id)]).expect("summary");
    let reserved: f64 = list(f, "mantle.product.issuance.AssetReservation", &[("assetId", asset_id)])
        .iter()
        .map(|r| number(r.get("quantity")))
        .sum();
    assert_eq!(
        number(summary.get("quantityOnHandTotal")) - number(summary.get("availableToPromiseTotal")),
        reserved,
        "QOH - ATP = reservations for {asset_id}"
    );
}

fn o2c_create_inventory_order(f: &Flow, st: &mut O2c) {
    login("john.doe");
    let add = call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &[
            ("productId", s("DEMO_UNIT")),
            ("quantity", n("10")),
            ("customerPartyId", s("CustJqp")),
            ("currencyUomId", s("USD")),
            ("productStoreId", s("POPC_DEFAULT")),
        ],
    );
    st.inventory_order_id = need(&add, "orderId");
    let oid = st.inventory_order_id.clone();
    call(
        "mantle.order.OrderServices.set#OrderBillingShippingInfo",
        &[
            ("orderId", s(&oid)),
            ("paymentMethodId", s("CustJqpCc")),
            ("shippingPostalContactMechId", s("CustJqpAddr")),
            ("shippingTelecomContactMechId", s("CustJqpTeln")),
            ("carrierPartyId", s("_NA_")),
            ("shipmentMethodEnumId", s("ShMthGround")),
        ],
    );
    call(
        "mantle.order.OrderServices.place#Order",
        &[("orderId", s(&oid)), ("requireInventory", Value::Bool(false))],
    );
    call("mantle.order.OrderServices.approve#Order", &[("orderId", s(&oid))]);
    let xml = format!(
        r#"<entity-facade-xml>
        <mantle.order.OrderHeader orderId="{oid}" statusId="OrderApproved" currencyUomId="USD" productStoreId="POPC_DEFAULT" grandTotal="10.00"/>
        <mantle.order.OrderPart orderId="{oid}" orderPartSeqId="01" vendorPartyId="ORG_ZIZI_RETAIL" customerPartyId="CustJqp"
            shipmentMethodEnumId="ShMthGround" postalContactMechId="CustJqpAddr" telecomContactMechId="CustJqpTeln" partTotal="10.00"/>
        <mantle.order.OrderItem orderId="{oid}" orderItemSeqId="01" orderPartSeqId="01" itemTypeEnumId="ItemProduct"
            productId="DEMO_UNIT" itemDescription="Demo Product One Unit" quantity="10" unitAmount="1.00" isModifiedPrice="N"/>
        <mantle.product.issuance.AssetReservation orderId="{oid}" orderItemSeqId="01" quantity="10" quantityNotAvailable="0"
            quantityNotIssued="10" assetId="DEMO_UNITA" productId="DEMO_UNIT" sequenceNum="0" reservationOrderEnumId="AsResOrdFifoRec"/>
    </entity-facade-xml>"#
    );
    assert_clean(check(f, &xml), "create Inventory Tests Sales Order");
    reservation_invariant(f, "DEMO_UNITA");
}

fn o2c_ship_inventory_and_cancel(f: &Flow, st: &mut O2c, partial: bool) {
    let ship = call(
        "mantle.shipment.ShipmentServices.create#OrderPartShipment",
        &[
            ("orderId", s(&st.inventory_order_id)),
            ("orderPartSeqId", s("01")),
            ("createPackage", Value::Bool(true)),
        ],
    );
    let sid = need(&ship, "shipmentId");
    let package = need(&ship, "shipmentPackageSeqId");
    let pack = |qty: &str| {
        call(
            "mantle.shipment.ShipmentServices.pack#ShipmentProduct",
            &[
                ("productId", s("DEMO_UNIT")),
                ("quantity", n(qty)),
                ("shipmentId", s(&sid)),
                ("shipmentPackageSeqId", s(&package)),
            ],
        );
    };
    if partial {
        pack("5");
        for issuance in list(f, "mantle.product.issuance.AssetIssuance", &[("shipmentId", &sid)]) {
            call(
                "mantle.shipment.ShipmentServices.unpack#ShipmentItemIssuance",
                &[("assetIssuanceId", s(&text(&issuance, "assetIssuanceId")))],
            );
        }
        pack("5");
    } else {
        pack("10");
    }
    call(
        "mantle.shipment.ShipmentServices.pack#Shipment",
        &[("shipmentId", s(&sid))],
    );
    call(
        "mantle.shipment.ShipmentServices.ship#Shipment",
        &[("shipmentId", s(&sid))],
    );
    call(
        "mantle.shipment.ShipmentServices.cancel#Shipment",
        &[("shipmentId", s(&sid))],
    );
    let shipment = one(f, "mantle.shipment.Shipment", &[("shipmentId", &sid)]).unwrap();
    assert_eq!(text(&shipment, "statusId"), "ShipCancelled");
    reservation_invariant(f, "DEMO_UNITA");
}

fn o2c_reserve_with_displace(f: &Flow) {
    // The orders come from AssetReservationMultipleThreads (base id 53000); without that flow's data the step has
    // nothing to displace, so it ports as written and fails at the missing reservation.
    let before = list(f, "mantle.product.issuance.AssetReservation", &[("assetId", "55400")]);
    let sixty = before
        .iter()
        .find(|r| number(r.get("quantity")) == 60.0)
        .unwrap_or_else(|| {
            panic!("no 60 unit reservation on 55400 (AssetReservationMultipleThreads must run first): {before:?}")
        });
    let (order_id, item) = (text(sixty, "orderId"), text(sixty, "orderItemSeqId"));
    call(
        "mantle.product.AssetServices.reserve#AssetForOrderItem",
        &[
            ("orderId", s(&order_id)),
            ("orderItemSeqId", s(&item)),
            ("assetId", s("DEMO_1_1A")),
            ("resetReservations", Value::Bool(true)),
        ],
    );
    let after = list(
        f,
        "mantle.product.issuance.AssetReservation",
        &[("orderId", &order_id), ("orderItemSeqId", &item)],
    );
    let on_a = after
        .iter()
        .find(|r| text(r, "assetId") == "DEMO_1_1A")
        .expect("reserved against DEMO_1_1A");
    assert_eq!(number(on_a.get("quantity")), 60.0);
}

/// The order id of OrderToCashBasicFlow's first sales order, which ReturnToResponseBasicFlow returns items from
/// (`originalOrderId = "55500"` in the Spock file, a pinned sequenced id).
static O2C_ORDER: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

fn o2c_core(f: &Flow, st: &mut O2c) {
    o2c_create_sales_order(f, st);
    o2c_validate_asset_reservation(f, st);
    o2c_ship_sales_order(f, st);
    o2c_close_partial_filled_order(f, st);
    *O2C_ORDER.lock().unwrap() = Some((st.order_id.clone(), st.shipment_id.clone()));
}

fn flow_order_to_cash_basic(f: &Flow) {
    // The suite runs AssetReservationMultipleThreads and OrderProcureToPayBasicFlow first. OrderToCashBasicFlow
    // depends on them: CustJqp must already have placed orders (otherwise the 20% new customer promotion adds a
    // discount item to the first order), the purchase order puts the DEMO_1_1 stock in asset 55400, and the last
    // feature displaces one of the reservations the threads made.
    ensure_procure_to_pay_flow(f);
    let mut st = O2c::default();
    o2c_core(f, &mut st);
    o2c_validate_asset_issuance(f, &mut st);
    o2c_validate_issuance_accounting(f, &mut st);
    o2c_validate_shipment_invoice(f, &mut st);
    o2c_validate_invoice_and_payment_accounting(f, &mut st);
    o2c_create_and_ship_business_customer_order(f, &mut st);
    o2c_credit_memo(f, &mut st);
    o2c_apply_credit_memo(f, &mut st);
    o2c_receive_overpayment(f, &mut st);
    o2c_refund_overpayment(f, &mut st);
    o2c_create_inventory_order(f, &mut st);
    o2c_ship_inventory_and_cancel(f, &mut st, false);
    o2c_ship_inventory_and_cancel(f, &mut st, true);
    o2c_reserve_with_displace(f);
}

/// The sales order the return flow works on: OrderToCashBasicFlow's, run first if it has not.
fn original_order(f: &Flow) -> String {
    let known = O2C_ORDER.lock().unwrap().clone();
    if let Some((order, _)) = known {
        return order;
    }
    let mut st = O2c::default();
    o2c_core(f, &mut st);
    st.order_id
}

// ---------------------------------------------------------------------------------------------------------------
// AccountingActivities
// ---------------------------------------------------------------------------------------------------------------

/// Posted debits and credits of a GL account in a time period, 0/0 before the first posting.
fn gl_period(f: &Flow, gl_account: &str, period: &str, org: &str) -> (f64, f64) {
    let rows = list(
        f,
        "mantle.ledger.account.GlAccountOrgTimePeriod",
        &[
            ("glAccountId", gl_account),
            ("timePeriodId", period),
            ("organizationPartyId", org),
        ],
    );
    rows.first().map_or((0.0, 0.0), |r| {
        (number(r.get("postedDebits")), number(r.get("postedCredits")))
    })
}

fn acct_trans(org: &str, kind: &str, entries: &[(&str, &str, &str)]) -> String {
    let trans = call(
        "mantle.ledger.LedgerServices.create#AcctgTrans",
        &[
            ("acctgTransTypeEnumId", s(kind)),
            ("organizationPartyId", s(org)),
            ("amountUomId", s("USD")),
        ],
    );
    let id = need(&trans, "acctgTransId");
    for (gl, flag, amount) in entries {
        call(
            "mantle.ledger.LedgerServices.create#AcctgTransEntry",
            &[
                ("acctgTransId", s(&id)),
                ("glAccountId", s(gl)),
                ("debitCreditFlag", s(flag)),
                ("amount", n(amount)),
            ],
        );
    }
    call(
        "mantle.ledger.LedgerServices.post#AcctgTrans",
        &[("acctgTransId", s(&id))],
    );
    id
}

fn flow_accounting_activities(f: &Flow) {
    const ORG: &str = "ORG_ZIZI_RETAIL";
    const ORG2: &str = "ORG_ZIZI_CORP";
    login("john.doe");
    let period = |org: &str| -> String {
        let out = call(
            "mantle.ledger.LedgerServices.get#OrganizationFiscalTimePeriods",
            &[
                ("organizationPartyId", s(org)),
                ("filterDate", s(&now_text())),
                ("timePeriodTypeId", s("FiscalMonth")),
            ],
        );
        match out.get("timePeriodList") {
            Some(Value::List(items)) => match items.first() {
                Some(Value::Map(m)) => moqui_lib::object::to_plain_string(m.get("timePeriodId").expect("timePeriodId")),
                _ => panic!("no fiscal month for {org}: {out:?}"),
            },
            _ => panic!("no timePeriodList for {org}: {out:?}"),
        }
    };
    let (tp, tp2) = (period(ORG), period(ORG2));
    // the Spock spec starts from empty ledgers; the balances here are deltas from what the earlier flows posted
    let before = |gl: &str| gl_period(f, gl, &tp, ORG);
    let (d111, c111) = before("111100000");
    let (_, c3311) = before("331100000");
    let (_, c3321) = before("332100000");
    let (d111b, _) = gl_period(f, "111100000", &tp2, ORG2);

    // initial Investment AcctgTrans
    let first = acct_trans(
        ORG,
        "AttCapitalization",
        &[("111100000", "D", "100000"), ("331100000", "C", "100000")],
    );
    acct_trans(
        ORG,
        "AttCapitalization",
        &[
            ("111100000", "D", "125000"),
            ("331100000", "C", "100000"),
            ("332100000", "C", "25000"),
        ],
    );
    acct_trans(
        ORG2,
        "AttCapitalization",
        &[("111100000", "D", "150000"), ("331100000", "C", "150000")],
    );
    let (d, _) = gl_period(f, "111100000", &tp, ORG);
    assert_eq!(d - d111, 225000.0, "postedDebits 111100000");
    assert_eq!(gl_period(f, "331100000", &tp, ORG).1 - c3311, 200000.0);
    assert_eq!(gl_period(f, "332100000", &tp, ORG).1 - c3321, 25000.0);
    assert_eq!(gl_period(f, "111100000", &tp2, ORG2).0 - d111b, 150000.0);
    let _ = c111;

    // unpost and Check GlAccountOrgTimePeriod
    call(
        "mantle.ledger.LedgerServices.unpost#AcctgTrans",
        &[("acctgTransId", s(&first))],
    );
    assert_eq!(
        text(
            &one(f, "mantle.ledger.transaction.AcctgTrans", &[("acctgTransId", &first)]).unwrap(),
            "isPosted"
        ),
        "N"
    );
    assert_eq!(
        gl_period(f, "111100000", &tp, ORG).0 - d111,
        125000.0,
        "unposted 100000"
    );
    // repost
    call(
        "mantle.ledger.LedgerServices.post#AcctgTrans",
        &[("acctgTransId", s(&first))],
    );
    assert_eq!(
        text(
            &one(f, "mantle.ledger.transaction.AcctgTrans", &[("acctgTransId", &first)]).unwrap(),
            "isPosted"
        ),
        "Y"
    );
    assert_eq!(gl_period(f, "111100000", &tp, ORG).0 - d111, 225000.0, "reposted");

    // record Retained Earnings and Dividends Distributable AcctgTrans
    acct_trans(
        ORG,
        "AttPeriodClosing",
        &[("850000000", "D", "1000"), ("335000000", "C", "1000")],
    );
    acct_trans(
        ORG,
        "AttPeriodClosing",
        &[("335000000", "D", "600"), ("333100000", "C", "600")],
    );
    // pay Dividends AcctgTrans
    acct_trans(
        ORG,
        "AttDisbursement",
        &[("333100000", "D", "300"), ("111100000", "C", "300")],
    );
    acct_trans(
        ORG,
        "AttDisbursement",
        &[("335000000", "D", "50000"), ("111100000", "C", "50000")],
    );

    // expense Invoice
    let invoice = call(
        "mantle.account.InvoiceServices.create#Invoice",
        &[
            ("fromPartyId", s("ZiddlemanInc")),
            ("toPartyId", s(ORG)),
            ("currencyUomId", s("USD")),
        ],
    );
    let invoice_id = need(&invoice, "invoiceId");
    let items = [
        ("ItemExpCommTelephone", "Land Line Service", "123.45"),
        ("ItemExpCommCellular", "Cell Service", "234.56"),
        ("ItemExpCommNetwork", "Internet Service", "345.67"),
        ("ItemExpHosting", "Site Hosting", "45.45"),
        ("ItemExpOfficeSup", "Office Supplies", "101.01"),
        ("ItemExpUtilHeating", "Heating", "33.33"),
        ("ItemExpUtilElec", "Electricity", "111.11"),
        ("ItemExpUtilWater", "Water", "30.00"),
        ("ItemExpUtilTrash", "Trash Disposal", "34.00"),
        ("ItemExpInterestEquip", "Equipment Interest", "321.12"),
        ("ItemExpInterestReal", "Real Estate Interest", "444.55"),
    ];
    for (kind, description, amount) in items {
        call(
            "mantle.account.InvoiceServices.create#InvoiceItem",
            &[
                ("invoiceId", s(&invoice_id)),
                ("itemTypeEnumId", s(kind)),
                ("description", s(description)),
                ("quantity", n("1")),
                ("amount", n(amount)),
            ],
        );
    }
    call(
        "update#mantle.account.invoice.Invoice",
        &[("invoiceId", s(&invoice_id)), ("statusId", s("InvoiceReceived"))],
    );
    call(
        "update#mantle.account.invoice.Invoice",
        &[("invoiceId", s(&invoice_id)), ("statusId", s("InvoiceApproved"))],
    );
    let inv = one(f, "mantle.account.invoice.Invoice", &[("invoiceId", &invoice_id)]).unwrap();
    assert_eq!(text(&inv, "statusId"), "InvoiceApproved");
    assert_eq!(text(&inv, "acctgTransResultEnumId"), "AtrSuccess");
    let trans = list(f, "mantle.ledger.transaction.AcctgTrans", &[("invoiceId", &invoice_id)]);
    assert_eq!(trans.len(), 1, "{trans:?}");
    assert_eq!(text(&trans[0], "acctgTransTypeEnumId"), "AttPurchaseInvoice");
    let entries = list(
        f,
        "mantle.ledger.transaction.AcctgTransEntry",
        &[("acctgTransId", &text(&trans[0], "acctgTransId"))],
    );
    assert_eq!(entries.len(), 12, "11 expense debits and the payable credit");
    let payable = entries
        .iter()
        .find(|e| text(e, "glAccountId") == "212000000")
        .expect("payable entry");
    assert_eq!(number(payable.get("amount")), 1824.25);
}

/// The current time in UTC as text, the form a Timestamp parameter takes (the fixture's user time zone is UTC).
fn utc_now_text() -> String {
    jiff::Timestamp::now().strftime("%Y-%m-%d %H:%M:%S").to_string()
}

fn now_text() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S").to_string()
}

/// A thruDate after rows just written: `now_text` drops the milliseconds, so it can fall before them. Spock uses
/// `effectiveTime + 1`.
fn after_now_text() -> String {
    // UTC: the user's time zone in the fixture is UTC, while `Zoned::now` is local time
    (jiff::Timestamp::now() + jiff::Span::new().seconds(2))
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string()
}

// ---------------------------------------------------------------------------------------------------------------
// AssetReservationMultipleThreads
// ---------------------------------------------------------------------------------------------------------------

fn make_threaded_order(f: &Flow, thread: usize) -> String {
    let context = f.factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    login("joe@public.com");
    let products = ["DEMO_1_1", "DEMO_UNIT"];
    let (first, second) = if thread.is_multiple_of(2) { (0, 1) } else { (1, 0) };
    let common = |extra: Vec<(&'static str, Value)>| {
        let mut params = vec![
            ("quantity", n("60")),
            ("customerPartyId", s("CustJqp")),
            ("currencyUomId", s("USD")),
            ("productStoreId", s("POPC_DEFAULT")),
            ("requireInventory", Value::Bool(false)),
        ];
        params.extend(extra);
        params
    };
    let add1 = call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &common(vec![("productId", s(products[first]))]),
    );
    let order_id = need(&add1, "orderId");
    let part = need(&add1, "orderPartSeqId");
    call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &common(vec![
            ("orderId", s(&order_id)),
            ("orderPartSeqId", s(&part)),
            ("productId", s(products[second])),
        ]),
    );
    call(
        "mantle.order.OrderServices.set#OrderBillingShippingInfo",
        &[
            ("orderId", s(&order_id)),
            ("paymentMethodId", s("CustJqpCc")),
            ("paymentInstrumentEnumId", s("PiCreditCard")),
            ("shippingPostalContactMechId", s("CustJqpAddr")),
            ("shippingTelecomContactMechId", s("CustJqpTeln")),
            ("carrierPartyId", s("_NA_")),
            ("shipmentMethodEnumId", s("ShMthGround")),
        ],
    );
    // place#Order triggers the asset reservation
    call(
        "mantle.order.OrderServices.place#Order",
        &[("orderId", s(&order_id)), ("requireInventory", Value::Bool(false))],
    );
    logout();
    order_id
}

static ASSET_RESERVATION_DONE: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

/// Run AssetReservationMultipleThreads once per process, as the suite does before the flows that need its orders.
fn ensure_asset_reservation_flow(f: &Flow) {
    let done = *ASSET_RESERVATION_DONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !done {
        flow_asset_reservation_multiple_threads(f);
    }
}

fn flow_asset_reservation_multiple_threads(f: &Flow) {
    *ASSET_RESERVATION_DONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    login("john.doe");
    let atp = |f: &Flow| -> f64 {
        list(f, "mantle.product.asset.Asset", &[("productId", "DEMO_1_1")])
            .iter()
            .map(|a| number(a.get("availableToPromiseTotal")))
            .sum()
    };
    let _initial = atp(f);
    let handles: Vec<_> = (0..5)
        .map(|thread| {
            let f = Flow {
                facade: f.facade.clone(),
                factory: f.factory.clone(),
            };
            std::thread::spawn(move || make_threaded_order(&f, thread))
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join()).collect();
    assert!(
        results.iter().all(Result::is_ok),
        "an order thread panicked, see its SERVICE line above"
    );
    let orders: Vec<String> = results.into_iter().map(Result::unwrap).collect();
    assert_eq!(orders.len(), 5);
    let after = atp(f);
    let not_available: f64 = list(
        f,
        "mantle.product.issuance.AssetReservation",
        &[("productId", "DEMO_1_1")],
    )
    .iter()
    .map(|r| number(r.get("quantityNotAvailable")))
    .sum();
    assert_eq!(
        after, -not_available,
        "ATP of DEMO_1_1 equals minus the quantity not available"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// OrderToCashTime
// ---------------------------------------------------------------------------------------------------------------

fn flow_order_to_cash_time(f: &Flow) {
    let _ = f;
    let started = std::time::Instant::now();
    for i in 0..5 {
        login("joe@public.com");
        let common = [
            ("customerPartyId", s("CustJqp")),
            ("currencyUomId", s("USD")),
            ("productStoreId", s("POPC_DEFAULT")),
        ];
        let mut first = vec![("productId", s("DEMO_1_1")), ("quantity", n("1"))];
        first.extend(common.iter().cloned());
        let add1 = call("mantle.order.OrderServices.add#OrderProductQuantity", &first);
        let order_id = need(&add1, "orderId");
        let part = need(&add1, "orderPartSeqId");
        for (product, require) in [("DEMO_3_1", true), ("DEMO_2_1", false)] {
            let mut params = vec![
                ("orderId", s(&order_id)),
                ("productId", s(product)),
                ("quantity", n("1")),
            ];
            params.extend(common.iter().cloned());
            if !require {
                params.push(("requireInventory", Value::Bool(false)));
            }
            call("mantle.order.OrderServices.add#OrderProductQuantity", &params);
        }
        call(
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            &[
                ("orderId", s(&order_id)),
                ("paymentMethodId", s("CustJqpCc")),
                ("shippingPostalContactMechId", s("CustJqpAddr")),
                ("shippingTelecomContactMechId", s("CustJqpTeln")),
                ("carrierPartyId", s("_NA_")),
                ("shipmentMethodEnumId", s("ShMthGround")),
            ],
        );
        call(
            "mantle.order.OrderServices.place#Order",
            &[("orderId", s(&order_id)), ("requireInventory", Value::Bool(false))],
        );
        logout();
        login("john.doe");
        call("mantle.order.OrderServices.approve#Order", &[("orderId", s(&order_id))]);
        call(
            "mantle.shipment.ShipmentServices.ship#OrderPart",
            &[("orderId", s(&order_id)), ("orderPartSeqId", s(&part))],
        );
        logout();
        eprintln!(
            "[{}/5 - {}ms] created and shipped order {order_id}",
            i + 1,
            started.elapsed().as_millis()
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// OrderTenantAccess (not run by the Spock suite; kept as a placeholder there)
// ---------------------------------------------------------------------------------------------------------------

fn flow_order_tenant_access(f: &Flow) {
    login("joe@public.com");
    let add = call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &[
            ("productId", s("DEMO_TNT")),
            ("quantity", n("3")),
            ("customerPartyId", s("CustJqp")),
            ("currencyUomId", s("USD")),
            ("productStoreId", s("POPC_DEFAULT")),
        ],
    );
    let order_id = need(&add, "orderId");
    let info = call(
        "mantle.order.OrderServices.set#OrderBillingShippingInfo",
        &[
            ("orderId", s(&order_id)),
            ("paymentMethodId", s("CustJqpCc")),
            ("shippingPostalContactMechId", s("CustJqpAddr")),
            ("shippingTelecomContactMechId", s("CustJqpTeln")),
            ("carrierPartyId", s("_NA_")),
            ("shipmentMethodEnumId", s("ShMthGround")),
        ],
    );
    // place order, triggers tenant provision, etc
    call("mantle.order.OrderServices.place#Order", &[("orderId", s(&order_id))]);
    logout();
    let payment_id = need(&info, "paymentId");
    let xml = format!(
        r#"<entity-facade-xml>
        <mantle.order.OrderHeader orderId="{order_id}" statusId="OrderCompleted" currencyUomId="USD" productStoreId="POPC_DEFAULT" grandTotal="29.97"/>
        <mantle.account.payment.Payment paymentId="{payment_id}" paymentTypeEnumId="PtInvoicePayment" paymentMethodId="CustJqpCc"
            paymentInstrumentEnumId="PiCreditCard" orderId="{order_id}" orderPartSeqId="01" statusId="PmntDelivered" amount="29.97"
            amountUomId="USD" fromPartyId="CustJqp" toPartyId="ORG_ZIZI_RETAIL"/>
        <mantle.order.OrderItem orderId="{order_id}" orderItemSeqId="01" orderPartSeqId="01" itemTypeEnumId="ItemProduct"
            productId="DEMO_TNT" itemDescription="Demo Tenant 1 Month Subscription" quantity="3" isModifiedPrice="N"/>
    </entity-facade-xml>"#
    );
    assert_clean(check(f, &xml), "OrderTenantAccess create Sales Order");
    let billings = list(f, "mantle.order.OrderItemBilling", &[("orderId", &order_id)]);
    assert_eq!(billings.len(), 1, "the invoice created on completion: {billings:?}");
    let invoice_id = text(&billings[0], "invoiceId");
    let invoice = one(f, "mantle.account.invoice.Invoice", &[("invoiceId", &invoice_id)]).unwrap();
    assert_eq!(text(&invoice, "statusId"), "InvoicePmtRecvd");
    let trans = list(f, "mantle.ledger.transaction.AcctgTrans", &[("invoiceId", &invoice_id)]);
    assert_eq!(text(&trans[0], "acctgTransTypeEnumId"), "AttSalesInvoice");
    let pay = list(f, "mantle.ledger.transaction.AcctgTrans", &[("paymentId", &payment_id)]);
    assert_eq!(text(&pay[0], "acctgTransTypeEnumId"), "AttIncomingPayment");
}

// ---------------------------------------------------------------------------------------------------------------
// ReturnToResponseBasicFlow
// ---------------------------------------------------------------------------------------------------------------

fn flow_return_to_response(f: &Flow) {
    let original = original_order(f);
    login("joe@public.com");
    let created = call(
        "mantle.order.ReturnServices.create#ReturnFromOrder",
        &[("orderId", s(&original)), ("orderPartSeqId", s("01"))],
    );
    let return_id = need(&created, "returnId");
    for (item, qty, reason, response) in [
        ("01", "1", "RrsnMisShip", "RrspReplace"),
        ("02", "2", "RrsnDefective", "RrspRefund"),
        ("03", "3", "RrsnDidNotWant", "RrspCredit"),
    ] {
        call(
            "mantle.order.ReturnServices.add#OrderItemToReturn",
            &[
                ("returnId", s(&return_id)),
                ("orderId", s(&original)),
                ("orderItemSeqId", s(item)),
                ("returnQuantity", n(qty)),
                ("returnReasonEnumId", s(reason)),
                ("returnResponseEnumId", s(response)),
            ],
        );
    }
    call(
        "mantle.order.ReturnServices.request#Return",
        &[("returnId", s(&return_id))],
    );
    logout();
    let ret = one(f, "mantle.order.return.Return", &[("returnId", &return_id)]).unwrap();
    assert_eq!(text(&ret, "statusId"), "ReturnRequested");

    // approve Return
    login("john.doe");
    call(
        "mantle.order.ReturnServices.approve#Return",
        &[("returnId", s(&return_id))],
    );
    assert_eq!(
        text(
            &one(f, "mantle.order.return.Return", &[("returnId", &return_id)]).unwrap(),
            "statusId"
        ),
        "ReturnApproved"
    );

    // receive Return Shipment
    let sources = list(f, "mantle.shipment.ShipmentItemSource", &[("returnId", &return_id)]);
    assert!(!sources.is_empty(), "approve#Return creates the return shipment");
    let shipment_id = text(&sources[0], "shipmentId");
    for (product, qty) in [("DEMO_1_1", "1.0"), ("DEMO_2_1", "2.0"), ("DEMO_3_1", "2.0")] {
        call(
            "mantle.shipment.ShipmentServices.receive#ShipmentProduct",
            &[
                ("shipmentId", s(&shipment_id)),
                ("productId", s(product)),
                ("statusId", s("AstOnHold")),
                ("quantityAccepted", n(qty)),
            ],
        );
    }
    call(
        "mantle.shipment.ShipmentServices.deliver#Shipment",
        &[("shipmentId", s(&shipment_id))],
    );

    // validate Return Completed
    let ret = one(f, "mantle.order.return.Return", &[("returnId", &return_id)]).unwrap();
    assert_eq!(text(&ret, "statusId"), "ReturnCompleted");
    let items = list(f, "mantle.order.return.ReturnItem", &[("returnId", &return_id)]);
    assert!(
        items.iter().all(|i| text(i, "statusId") == "ReturnCompleted"),
        "{items:?}"
    );
    let replace = items
        .iter()
        .find(|i| text(i, "returnItemSeqId") == "01")
        .expect("item 01");
    let replacement_order = text(replace, "replacementOrderId");
    assert!(!replacement_order.is_empty(), "replacement order created: {replace:?}");
    let refund = items
        .iter()
        .find(|i| text(i, "orderItemSeqId") == "02")
        .expect("refund item");
    assert!(
        !text(refund, "refundPaymentId").is_empty(),
        "refund payment: {refund:?}"
    );
    assert_eq!(number(refund.get("responseAmount")), 15.54);
    let credit = items
        .iter()
        .find(|i| text(i, "orderItemSeqId") == "03")
        .expect("credit item");
    assert!(
        !text(credit, "finAccountTransId").is_empty(),
        "customer credit transaction: {credit:?}"
    );
    assert_eq!(number(credit.get("responseAmount")), 24.24);

    // validate Replacement Order and Asset Reservation
    let replacement =
        one(f, "mantle.order.OrderHeader", &[("orderId", &replacement_order)]).expect("replacement order");
    assert_eq!(text(&replacement, "statusId"), "OrderApproved");
    assert!(!list(
        f,
        "mantle.product.issuance.AssetReservation",
        &[("orderId", &replacement_order)]
    )
    .is_empty());

    // ship Replacement Order
    call(
        "mantle.shipment.ShipmentServices.ship#OrderPart",
        &[("orderId", s(&replacement_order)), ("orderPartSeqId", s("01"))],
    );
    let replacement = one(f, "mantle.order.OrderHeader", &[("orderId", &replacement_order)]).unwrap();
    assert_eq!(text(&replacement, "statusId"), "OrderCompleted");

    // purchase with return credit
    logout();
    login("joe@public.com");
    let accounts = list(
        f,
        "mantle.account.financial.FinancialAccount",
        &[("ownerPartyId", "CustJqp"), ("finAccountTypeId", "CustomerCredit")],
    );
    let fin_account_id = text(accounts.first().expect("customer credit account"), "finAccountId");
    let add = call(
        "mantle.order.OrderServices.add#OrderProductQuantity",
        &[
            ("productId", s("DEMO_3_1")),
            ("quantity", n("1")),
            ("customerPartyId", s("CustJqp")),
            ("productStoreId", s("POPC_DEFAULT")),
        ],
    );
    let order_id = need(&add, "orderId");
    let part = need(&add, "orderPartSeqId");
    call(
        "mantle.order.OrderServices.set#OrderBillingShippingInfo",
        &[
            ("orderId", s(&order_id)),
            ("shippingPostalContactMechId", s("CustJqpAddr")),
            ("shippingTelecomContactMechId", s("CustJqpTeln")),
            ("carrierPartyId", s("_NA_")),
            ("shipmentMethodEnumId", s("ShMthGround")),
            ("finAccountId", s(&fin_account_id)),
        ],
    );
    call("mantle.order.OrderServices.place#Order", &[("orderId", s(&order_id))]);
    logout();
    login("john.doe");
    call("mantle.order.OrderServices.approve#Order", &[("orderId", s(&order_id))]);
    call(
        "mantle.shipment.ShipmentServices.ship#OrderPart",
        &[("orderId", s(&order_id)), ("orderPartSeqId", s(&part))],
    );
    let withdraws = list(
        f,
        "mantle.account.financial.FinancialAccountTrans",
        &[
            ("finAccountId", &fin_account_id),
            ("finAccountTransTypeEnumId", "FattWithdraw"),
        ],
    );
    assert!(!withdraws.is_empty(), "the order paid from the customer credit account");
}

// ---------------------------------------------------------------------------------------------------------------
// OrderProcureToPayBasicFlow
// ---------------------------------------------------------------------------------------------------------------

static PROCURE_TO_PAY_DONE: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

/// Run OrderProcureToPayBasicFlow (after AssetReservationMultipleThreads) once per process.
fn ensure_procure_to_pay_flow(f: &Flow) {
    ensure_asset_reservation_flow(f);
    let done = *PROCURE_TO_PAY_DONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !done {
        flow_order_procure_to_pay(f);
    }
}

fn flow_order_procure_to_pay(f: &Flow) {
    ensure_asset_reservation_flow(f);
    *PROCURE_TO_PAY_DONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    const VENDOR: &str = "ZiddlemanInc";
    const CUSTOMER: &str = "ORG_ZIZI_RETAIL";
    const FACILITY: &str = "ZIRET_WH";
    login("john.doe");

    // create Purchase Order
    let price = |qty: &str| {
        call(
            "mantle.product.PriceServices.get#ProductPrice",
            &[
                ("productId", s("DEMO_1_1")),
                ("priceUomId", s("USD")),
                ("quantity", n(qty)),
                ("vendorPartyId", s(VENDOR)),
                ("customerPartyId", s(CUSTOMER)),
            ],
        )
    };
    assert_eq!(number(price("1").get("price")), 9.0);
    assert_eq!(number(price("100").get("price")), 8.0);
    let order = call(
        "mantle.order.OrderServices.create#Order",
        &[
            ("customerPartyId", s(CUSTOMER)),
            ("vendorPartyId", s(VENDOR)),
            ("currencyUomId", s("USD")),
            ("facilityId", s(FACILITY)),
        ],
    );
    let po = need(&order, "orderId");
    let part = need(&order, "orderPartSeqId");
    for (product, qty, kind, amount) in [
        ("DEMO_1_1", "400", "ItemInventory", None),
        ("DEMO_3_1", "100", "ItemInventory", None),
        ("EQUIP_1", "2", "ItemAsset", Some("10000")),
    ] {
        let mut params = vec![
            ("orderId", s(&po)),
            ("orderPartSeqId", s(&part)),
            ("productId", s(product)),
            ("quantity", n(qty)),
            ("itemTypeEnumId", s(kind)),
        ];
        if let Some(amount) = amount {
            params.push(("unitAmount", n(amount)));
        }
        call("mantle.order.OrderServices.add#OrderProductQuantity", &params);
    }
    call(
        "mantle.order.OrderServices.create#OrderItem",
        &[
            ("orderId", s(&po)),
            ("orderPartSeqId", s(&part)),
            ("unitAmount", n("145.00")),
            ("itemTypeEnumId", s("ItemShipping")),
            ("itemDescription", s("Incoming Freight")),
        ],
    );
    let info = call(
        "mantle.order.OrderServices.set#OrderBillingShippingInfo",
        &[
            ("orderId", s(&po)),
            ("orderPartSeqId", s(&part)),
            ("paymentMethodId", s("ZIRET_BA")),
            ("toPaymentMethodId", s("ZiddlemanInc_BA")),
            ("paymentInstrumentEnumId", s("PiAch")),
            ("shippingPostalContactMechId", s("ORG_ZIZI_RTL_SA")),
            ("shippingTelecomContactMechId", s("ORG_ZIZI_RTL_PT")),
            ("shipmentMethodEnumId", s("ShMthPickUp")),
        ],
    );
    let payment_id = need(&info, "paymentId");
    call("mantle.order.OrderServices.place#Order", &[("orderId", s(&po))]);
    call("mantle.order.OrderServices.approve#Order", &[("orderId", s(&po))]);
    let pay = one(f, "mantle.account.payment.Payment", &[("paymentId", &payment_id)]).unwrap();
    assert_eq!(text(&pay, "statusId"), "PmntPromised");
    assert_eq!(number(pay.get("amount")), 23795.0);

    // create Purchase Order Shipment and Schedule; set Shipment Shipped
    let ship = call(
        "mantle.shipment.ShipmentServices.create#OrderPartShipment",
        &[
            ("orderId", s(&po)),
            ("orderPartSeqId", s(&part)),
            ("destinationFacilityId", s(FACILITY)),
        ],
    );
    let shipment_id = need(&ship, "shipmentId");
    call(
        "mantle.shipment.ShipmentServices.ship#Shipment",
        &[("shipmentId", s(&shipment_id))],
    );

    // receive Purchase Order Shipment
    // depreciate over 5 years from the acquired date (today), as the Spock file adds 5 years to the effective time
    let eol_date = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .checked_add(jiff::Span::new().years(5))
        .expect("eol date");
    let eol = eol_date.to_string();
    let eol = eol.as_str();
    let receive = |extra: Vec<(&str, Value)>| {
        let mut params = vec![("shipmentId", s(&shipment_id)), ("facilityId", s(FACILITY))];
        params.extend(extra);
        call("mantle.shipment.ShipmentServices.receive#ShipmentProduct", &params)
    };
    receive(vec![
        ("productId", s("DEMO_1_1")),
        ("quantityAccepted", n("400")),
        ("locationSeqId", s("01010101")),
        ("lotNumber", s("A1111")),
    ]);
    receive(vec![
        ("productId", s("DEMO_3_1")),
        ("quantityAccepted", n("100")),
        ("locationSeqId", s("0801")),
        ("lotNumber", s("A2222")),
    ]);
    let equip1 = receive(vec![
        ("productId", s("EQUIP_1")),
        ("quantityAccepted", n("1")),
        ("serialNumber", s("PB2000AZQRTFP")),
        ("expectedEndOfLife", s(eol)),
        ("salvageValue", n("1500")),
        ("depreciationTypeEnumId", s("DtpDoubleDeclining")),
    ]);
    let equip2 = receive(vec![
        ("productId", s("EQUIP_1")),
        ("quantityAccepted", n("1")),
        ("serialNumber", s("PB2000GRWADRE")),
        ("expectedEndOfLife", s(eol)),
        ("salvageValue", n("1500")),
        ("depreciationTypeEnumId", s("DtpStraightLine")),
    ]);
    let first_asset = |out: &Map| match out.get("assetIdList") {
        Some(Value::List(ids)) => ids.first().map(moqui_lib::object::to_plain_string).unwrap_or_default(),
        _ => String::new(),
    };
    let (equip1_asset, equip2_asset) = (first_asset(&equip1), first_asset(&equip2));
    assert!(!equip1_asset.is_empty() && !equip2_asset.is_empty(), "receive results: {equip1:?} {equip2:?}");
    call(
        "mantle.shipment.ShipmentServices.deliver#Shipment",
        &[("shipmentId", s(&shipment_id))],
    );

    // complete Purchase Order (SECA on delivery)
    assert_eq!(
        text(
            &one(f, "mantle.order.OrderHeader", &[("orderId", &po)]).unwrap(),
            "statusId"
        ),
        "OrderCompleted"
    );
    // validate Assets Received
    let received = list(
        f,
        "mantle.product.receipt.AssetReceipt",
        &[("shipmentId", &shipment_id)],
    );
    assert_eq!(received.len(), 4, "{received:?}");
    assert_eq!(
        number(
            one(f, "mantle.product.asset.Asset", &[("assetId", &equip1_asset)])
                .unwrap()
                .get("acquireCost")
        ),
        10000.0
    );

    // move Received Inventory
    call(
        "mantle.product.AssetServices.move#Product",
        &[
            ("productId", s("DEMO_3_1")),
            ("quantity", n("100")),
            ("quantityAccepted", n("100")),
            ("facilityId", s(FACILITY)),
            ("locationSeqId", s("0801")),
            ("toLocationSeqId", s("01020101")),
        ],
    );

    // process Purchase Invoice: the SECA on shipment delivery created it from the order part
    let billings = list(f, "mantle.order.OrderItemBilling", &[("orderId", &po)]);
    assert!(!billings.is_empty(), "purchase invoice created on delivery");
    let invoice_id = text(&billings[0], "invoiceId");
    let total = call(
        "mantle.account.InvoiceServices.get#InvoiceTotal",
        &[("invoiceId", s(&invoice_id))],
    );
    assert_eq!(number(total.get("invoiceTotal")), 23795.0, "{total:?}");

    // approve Purchase Invoice
    call(
        "update#mantle.account.invoice.Invoice",
        &[("invoiceId", s(&invoice_id)), ("statusId", s("InvoiceApproved"))],
    );
    assert!(!list(f, "mantle.ledger.transaction.AcctgTrans", &[("invoiceId", &invoice_id)]).is_empty());

    // adjust Purchase Invoice
    call(
        "mantle.account.InvoiceServices.adjust#Invoice",
        &[
            ("invoiceId", s(&invoice_id)),
            ("description", s("")),
            ("amount", n("50")),
            ("itemTypeEnumId", s("ItemLateCharge")),
        ],
    );
    call(
        "mantle.account.InvoiceServices.adjust#Invoice",
        &[
            ("invoiceId", s(&invoice_id)),
            ("description", s("")),
            ("amount", n("-15")),
            ("itemTypeEnumId", s("ItemPromptDiscount")),
        ],
    );

    // send Purchase Invoice Payment (intentional overpay of 170.00)
    call(
        "update#mantle.account.payment.Payment",
        &[
            ("paymentId", s(&payment_id)),
            ("statusId", s("PmntAuthorized")),
            ("effectiveDate", s(&utc_now_text())),
            ("amount", n("24000.0")),
        ],
    );
    call(
        "mantle.account.NachaServices.generate#NachaFile",
        &[
            ("paymentMethodId", s("ZIRET_BA")),
            ("addOffsetRecord", Value::Bool(true)),
        ],
    );
    let apps = list(
        f,
        "mantle.account.payment.PaymentApplication",
        &[("paymentId", &payment_id), ("invoiceId", &invoice_id)],
    );
    assert_eq!(number(apps[0].get("amountApplied")), 23830.0);
    let pay = one(f, "mantle.account.payment.Payment", &[("paymentId", &payment_id)]).unwrap();
    assert_eq!(text(&pay, "statusId"), "PmntDelivered");
    assert_eq!(number(pay.get("unappliedTotal")), 170.0);

    // receive Supplier Overpay Refund
    let refund = call(
        "mantle.account.PaymentServices.create#Payment",
        &[
            ("paymentTypeEnumId", s("PtRefund")),
            ("statusId", s("PmntDelivered")),
            ("fromPartyId", s(VENDOR)),
            ("toPartyId", s(CUSTOMER)),
            ("effectiveDate", s(&utc_now_text())),
            ("paymentInstrumentEnumId", s("PiCompanyCheck")),
            ("amount", n("170.0")),
        ],
    );
    let applied = call(
        "mantle.account.PaymentServices.apply#PaymentToPayment",
        &[
            ("paymentId", s(&need(&refund, "paymentId"))),
            ("toPaymentId", s(&payment_id)),
        ],
    );
    assert_eq!(number(applied.get("amountApplied")), 170.0);

    // depreciate Fixed Assets
    let months = call(
        "mantle.ledger.LedgerServices.get#OrganizationFiscalTimePeriods",
        &[
            ("organizationPartyId", s(CUSTOMER)),
            ("filterDate", s(&now_text())),
            ("timePeriodTypeId", s("FiscalMonth")),
        ],
    );
    let time_period = match months.get("timePeriodList") {
        Some(Value::List(items)) => match items.first() {
            Some(Value::Map(m)) => moqui_lib::object::to_plain_string(m.get("timePeriodId").expect("timePeriodId")),
            _ => panic!("no fiscal month: {months:?}"),
        },
        _ => panic!("no timePeriodList: {months:?}"),
    };
    call(
        "mantle.ledger.AssetAutoPostServices.calculateAndPost#AllFixedAssetDepreciations",
        &[("timePeriodId", s(&time_period))],
    );
    let dep1 = one(
        f,
        "mantle.product.asset.AssetDepreciation",
        &[("assetId", &equip1_asset), ("timePeriodId", &time_period)],
    )
    .expect("depreciation 1");
    assert_eq!(number(dep1.get("monthlyDepreciation")), 283.33);
    let dep2 = one(
        f,
        "mantle.product.asset.AssetDepreciation",
        &[("assetId", &equip2_asset), ("timePeriodId", &time_period)],
    )
    .expect("depreciation 2");
    assert_eq!(number(dep2.get("monthlyDepreciation")), 141.67);

    // sell Depreciated Asset Loss (9000) and Gain (11000)
    for (asset, amount) in [(&equip1_asset, "9000"), (&equip2_asset, "11000")] {
        let created = call(
            "mantle.order.OrderServices.create#Order",
            &[
                ("customerPartyId", s("CustJqp")),
                ("vendorPartyId", s("ORG_ZIZI_RETAIL")),
                ("facilityId", s(FACILITY)),
            ],
        );
        let order_id = need(&created, "orderId");
        let first_part = need(&created, "orderPartSeqId");
        call(
            "mantle.order.OrderServices.set#OrderBillingShippingInfo",
            &[
                ("orderId", s(&order_id)),
                ("orderPartSeqId", s(&first_part)),
                ("shippingPostalContactMechId", s("CustJqpAddr")),
            ],
        );
        call(
            "mantle.order.OrderServices.add#OrderProductQuantity",
            &[
                ("orderId", s(&order_id)),
                ("orderPartSeqId", s(&first_part)),
                ("itemTypeEnumId", s("ItemAsset")),
                ("productId", s("EQUIP_1")),
                ("quantity", n("1")),
                ("unitAmount", n(amount)),
            ],
        );
        call("mantle.order.OrderServices.place#Order", &[("orderId", s(&order_id))]);
        call("mantle.order.OrderServices.approve#Order", &[("orderId", s(&order_id))]);
        let shipment = call(
            "mantle.shipment.ShipmentServices.create#OrderPartShipment",
            &[("orderId", s(&order_id)), ("orderPartSeqId", s(&first_part))],
        );
        let sid = need(&shipment, "shipmentId");
        call(
            "update#mantle.shipment.Shipment",
            &[("shipmentId", s(&sid)), ("statusId", s("ShipScheduled"))],
        );
        call(
            "mantle.shipment.ShipmentServices.pack#ShipmentProduct",
            &[
                ("shipmentId", s(&sid)),
                ("productId", s("EQUIP_1")),
                ("quantity", n("1")),
                ("assetId", s(asset)),
            ],
        );
        call(
            "mantle.shipment.ShipmentServices.pack#Shipment",
            &[("shipmentId", s(&sid))],
        );
        call(
            "mantle.shipment.ShipmentServices.ship#Shipment",
            &[("shipmentId", s(&sid))],
        );
        let sources = list(f, "mantle.shipment.ShipmentItemSource", &[("shipmentId", &sid)]);
        let invoice = text(&sources[0], "invoiceId");
        let total = call(
            "mantle.account.InvoiceServices.get#InvoiceTotal",
            &[("invoiceId", s(&invoice))],
        );
        let invoice_total = number(total.get("invoiceTotal"));
        call(
            "mantle.account.PaymentServices.create#InvoicePayment",
            &[
                ("invoiceId", s(&invoice)),
                ("statusId", s("PmntDelivered")),
                ("amountUomId", s("USD")),
                ("amount", n(&invoice_total.to_string())),
                ("paymentInstrumentEnumId", s("PiPersonalCheck")),
                ("effectiveDate", s(&utc_now_text())),
                ("paymentRefNum", s("123456")),
            ],
        );
        let after = call(
            "mantle.account.InvoiceServices.get#InvoiceTotal",
            &[("invoiceId", s(&invoice))],
        );
        assert_eq!(number(after.get("unpaidTotal")), 0.0);
        assert_eq!(number(after.get("appliedPaymentsTotal")), invoice_total);
    }

    // inventory Shrinkage and Found
    call(
        "mantle.product.AssetServices.record#PhysicalInventoryChange",
        &[
            ("productId", s("DEMO_1_1")),
            ("facilityId", s(FACILITY)),
            ("quantityChange", n("-10")),
            ("statusId", s("AstAvailable")),
            ("locationSeqId", s("01010101")),
            ("varianceReasonEnumId", s("InVrLost")),
            ("comments", s("Test lost 10 DEMO_1_1")),
        ],
    );
    let not_available: f64 = list(
        f,
        "mantle.product.issuance.AssetReservation",
        &[("productId", "DEMO_1_1")],
    )
    .iter()
    .map(|r| number(r.get("quantityNotAvailable")))
    .sum();
    assert_eq!(not_available, 0.0);
    call(
        "mantle.product.AssetServices.record#PhysicalInventoryChange",
        &[
            ("productId", s("DEMO_1_1")),
            ("facilityId", s(FACILITY)),
            ("quantityChange", n("10")),
            ("locationSeqId", s("01010201")),
            ("varianceReasonEnumId", s("InVrFound")),
            ("comments", s("Test found 10 DEMO_1_1")),
        ],
    );
}

// ---------------------------------------------------------------------------------------------------------------
// WorkPlanToCashBasicFlow
// ---------------------------------------------------------------------------------------------------------------

fn flow_work_plan_to_cash(f: &Flow) {
    const YEAR: &str = "2016";
    login("john.doe");
    let account = |first: &str, last: &str, email: &str, username: &str| {
        call(
            "mantle.party.PartyServices.create#Account",
            &[
                ("firstName", s(first)),
                ("lastName", s(last)),
                ("emailAddress", s(email)),
                ("username", s(username)),
                ("newPassword", s("moqui1!!")),
                ("newPasswordVerify", s("moqui1!!")),
                ("loginAfterCreate", Value::Bool(false)),
            ],
        )
    };
    let relate = |kind: &str, from: &str, from_role: &str, to: &str, to_role: &str| {
        call(
            "create#mantle.party.PartyRelationship",
            &[
                ("relationshipTypeEnumId", s(kind)),
                ("fromPartyId", s(from)),
                ("fromRoleTypeId", s(from_role)),
                ("toPartyId", s(to)),
                ("toRoleTypeId", s(to_role)),
                ("fromDate", s(&now_text())),
            ],
        );
    };

    // create Vendor
    let vendor = call(
        "mantle.party.PartyServices.create#Organization",
        &[("roleTypeId", s("Vendor")), ("organizationName", s("Test Vendor"))],
    );
    let vendor_id = need(&vendor, "partyId");
    call(
        "mantle.party.ContactServices.store#PartyContactInfo",
        &[
            ("partyId", s(&vendor_id)),
            ("postalContactMechPurposeId", s("PostalPayment")),
            ("telecomContactMechPurposeId", s("PhonePayment")),
            ("emailContactMechPurposeId", s("EmailPayment")),
            ("countryGeoId", s("USA")),
            ("postalCode", s("84057")),
            ("postalCodeExt", s("4605")),
            ("countryCode", s("1")),
            ("areaCode", s("801")),
            ("contactNumber", s("123-4567")),
            ("emailAddress", s("vendor.ar@test.com")),
        ],
    );
    call(
        "create#mantle.party.PartyRole",
        &[("partyId", s(&vendor_id)), ("roleTypeId", s("OrgInternal"))],
    );
    call(
        "mantle.ledger.LedgerServices.init#PartyAccountingConfiguration",
        &[
            ("sourcePartyId", s("DefaultSettings")),
            ("organizationPartyId", s(&vendor_id)),
            ("startYear", s(YEAR)),
        ],
    );
    let vendor_rep = need(
        &account("Vendor", "TestRep", "vendor.rep@test.com", "vendor.rep"),
        "partyId",
    );
    relate("PrtRepresentative", &vendor_rep, "Manager", &vendor_id, "Vendor");

    // create Worker and Rates
    let worker = need(&account("Test", "Worker", "worker@test.com", "worker"), "partyId");
    relate("PrtAgent", &worker, "Worker", &vendor_id, "Vendor");
    for (rate_type, purpose, amount, party) in [
        ("RatpStandard", "RaprClient", "60.00", Some(&worker)),
        ("RatpStandard", "RaprVendor", "40.00", Some(&worker)),
        ("RatpNoCharge", "RaprVendor", "40.00", None),
    ] {
        let mut params = vec![
            ("rateTypeEnumId", s(rate_type)),
            ("ratePurposeEnumId", s(purpose)),
            ("timePeriodUomId", s("TF_hr")),
            ("emplPositionClassId", s("Programmer")),
            ("fromDate", s("1265184000000")),
            ("rateAmount", s(amount)),
            ("rateCurrencyUomId", s("USD")),
        ];
        if let Some(party) = party {
            params.push(("partyId", s(party)));
        }
        call("create#mantle.humanres.rate.RateAmount", &params);
    }

    // create Client
    let client = call(
        "mantle.party.PartyServices.create#Organization",
        &[("roleTypeId", s("Customer")), ("organizationName", s("Test Client"))],
    );
    let client_id = need(&client, "partyId");
    call(
        "mantle.party.ContactServices.store#PartyContactInfo",
        &[
            ("partyId", s(&client_id)),
            ("postalContactMechPurposeId", s("PostalBilling")),
            ("telecomContactMechPurposeId", s("PhoneBilling")),
            ("emailContactMechPurposeId", s("EmailBilling")),
            ("countryGeoId", s("USA")),
            ("postalCode", s("89119")),
            ("postalCodeExt", s("5263")),
            ("countryCode", s("1")),
            ("areaCode", s("702")),
            ("contactNumber", s("123-4567")),
            ("emailAddress", s("client.ap@test.com")),
        ],
    );
    let client_rep = need(
        &account("Client", "TestRep", "client.rep@test.com", "client.rep"),
        "partyId",
    );
    relate(
        "PrtRepresentative",
        &client_rep,
        "ClientBilling",
        &client_id,
        "Customer",
    );

    // create TEST Project and Milestones
    call(
        "mantle.work.ProjectServices.create#Project",
        &[
            ("workEffortId", s("TEST")),
            ("workEffortName", s("Test Proj")),
            ("clientPartyId", s(&client_id)),
            ("vendorPartyId", s(&vendor_id)),
            ("totalClientCostAllowed", s("2000")),
            ("costUomId", s("USD")),
        ],
    );
    call(
        "mantle.work.ProjectServices.update#Project",
        &[
            ("workEffortId", s("TEST")),
            ("workEffortName", s("Test Project")),
            ("statusId", s("WeInProgress")),
        ],
    );
    call(
        "create#mantle.work.effort.WorkEffortParty",
        &[
            ("workEffortId", s("TEST")),
            ("partyId", s(&worker)),
            ("roleTypeId", s("Assignee")),
            ("emplPositionClassId", s("Programmer")),
            ("fromDate", s(&format!("{YEAR}-11-01"))),
            ("statusId", s("WeptAssigned")),
        ],
    );
    for (id, name, start, end, status) in [
        ("TEST-MS-01", "Test Milestone 1", "11-01", "11-30", "WeInProgress"),
        ("TEST-MS-02", "Test Milestone 2", "12-01", "12-31", "WeApproved"),
    ] {
        call(
            "mantle.work.ProjectServices.create#Milestone",
            &[
                ("rootWorkEffortId", s("TEST")),
                ("workEffortId", s(id)),
                ("workEffortName", s(name)),
                ("estimatedStartDate", s(&format!("{YEAR}-{start}"))),
                ("estimatedCompletionDate", s(&format!("{YEAR}-{end}"))),
                ("statusId", s(status)),
            ],
        );
    }

    // create TEST Project Tasks
    for (id, parent, name, status, priority, purpose, hours) in [
        ("TEST-001", None, "Test Task 1", "WeApproved", "3", "WepTask", "10"),
        (
            "TEST-001A",
            Some("TEST-001"),
            "Test Task 1A",
            "WeInPlanning",
            "4",
            "WepNewFeature",
            "2",
        ),
        (
            "TEST-001B",
            Some("TEST-001"),
            "Test Task 1B",
            "WeApproved",
            "4",
            "WepFix",
            "2",
        ),
    ] {
        let mut params = vec![
            ("rootWorkEffortId", s("TEST")),
            ("workEffortId", s(id)),
            ("milestoneWorkEffortId", s("TEST-MS-01")),
            ("workEffortName", s(name)),
            ("estimatedCompletionDate", s(&format!("{YEAR}-11-15"))),
            ("statusId", s(status)),
            ("assignToPartyId", s(&worker)),
            ("priority", n(priority)),
            ("purposeEnumId", s(purpose)),
            ("estimatedWorkTime", n(hours)),
            ("description", s("Task for the work plan flow")),
        ];
        if let Some(parent) = parent {
            params.push(("parentWorkEffortId", s(parent)));
        }
        call("mantle.work.TaskServices.create#Task", &params);
    }

    // record TimeEntries and complete Tasks
    for id in ["TEST-001", "TEST-001A", "TEST-001B"] {
        call(
            "mantle.work.TaskServices.update#Task",
            &[("workEffortId", s(id)), ("statusId", s("WeInProgress"))],
        );
    }
    let time = |id: &str, rate: &str, remaining: &str, extra: Vec<(&str, Value)>, comments: &str| {
        let mut params = vec![
            ("workEffortId", s(id)),
            ("partyId", s(&worker)),
            ("rateTypeEnumId", s(rate)),
            ("remainingWorkTime", n(remaining)),
            ("comments", s(comments)),
        ];
        params.extend(extra);
        call("mantle.work.TaskServices.add#TaskTime", &params);
    };
    time("TEST-001", "RatpStandard", "3", vec![("hours", n("6"))], "Did stuff");
    time(
        "TEST-001A",
        "RatpStandard",
        "1",
        vec![("hours", n("1.5")), ("breakHours", n("0.5"))],
        "Hours and break test, no from/thru dates",
    );
    time(
        "TEST-001B",
        "RatpStandard",
        "0.5",
        vec![
            ("fromDate", s(&format!("{YEAR}-11-03 12:00:00"))),
            ("thruDate", s(&format!("{YEAR}-11-03 15:00:00"))),
            ("breakHours", n("1")),
        ],
        "Break and from/thru dates test, hours calculated",
    );
    time(
        "TEST-001B",
        "RatpNoCharge",
        "0.5",
        vec![("hours", n("4")), ("breakHours", n("1"))],
        "No charge for this one because we like you",
    );
    for id in ["TEST-001", "TEST-001A", "TEST-001B"] {
        call(
            "mantle.work.TaskServices.update#Task",
            &[
                ("workEffortId", s(id)),
                ("statusId", s("WeComplete")),
                ("resolutionEnumId", s("WerCompleted")),
            ],
        );
    }
    let times = list(f, "mantle.work.time.TimeEntry", &[("workEffortId", "TEST-001B")]);
    assert_eq!(times.len(), 2, "{times:?}");
    let xml = format!(
        r#"<entity-facade-xml>
        <mantle.work.effort.WorkEffort workEffortId="TEST-001" resolutionEnumId="WerCompleted" statusId="WeComplete" estimatedWorkTime="10" remainingWorkTime="3" actualWorkTime="6"/>
        <mantle.work.effort.WorkEffort workEffortId="TEST-001A" resolutionEnumId="WerCompleted" statusId="WeComplete" estimatedWorkTime="2" remainingWorkTime="1" actualWorkTime="1.5"/>
        <mantle.work.effort.WorkEffort workEffortId="TEST-001B" resolutionEnumId="WerCompleted" statusId="WeComplete" estimatedWorkTime="2" remainingWorkTime="0.5" actualWorkTime="6"/>
        <mantle.work.time.TimeEntry partyId="{worker}" rateTypeEnumId="RatpStandard" hours="6" workEffortId="TEST-001"/>
        <mantle.work.time.TimeEntry partyId="{worker}" rateTypeEnumId="RatpStandard" hours="1.5" breakHours="0.5" workEffortId="TEST-001A"/>
        <mantle.work.time.TimeEntry partyId="{worker}" rateTypeEnumId="RatpStandard" hours="2" breakHours="1" workEffortId="TEST-001B"/>
        <mantle.work.time.TimeEntry partyId="{worker}" rateTypeEnumId="RatpNoCharge" hours="4" breakHours="1" workEffortId="TEST-001B"/>
        </entity-facade-xml>"#
    );
    assert_clean(check(f, &xml), "record TimeEntries and complete Tasks");

    // create Request and Task for Request
    let request = call(
        "mantle.request.RequestServices.create#Request",
        &[
            ("clientPartyId", s(&client_id)),
            ("assignToPartyId", s(&worker)),
            ("requestName", s("Test Request 1")),
            ("description", s("Description of Test Request 1")),
            ("priority", n("7")),
            ("requestTypeEnumId", s("RqtSupport")),
            ("statusId", s("ReqSubmitted")),
            ("responseRequiredDate", s(&format!("{YEAR}-11-15 15:00:00"))),
        ],
    );
    let request_id = need(&request, "requestId");
    call(
        "mantle.request.RequestServices.update#Request",
        &[("requestId", s(&request_id)), ("statusId", s("ReqReviewed"))],
    );
    let task = call(
        "mantle.work.TaskServices.create#Task",
        &[
            ("rootWorkEffortId", s("TEST")),
            ("workEffortName", s("Test Request 1 Task")),
            ("estimatedCompletionDate", s(&format!("{YEAR}-11-15"))),
            ("statusId", s("WeApproved")),
            ("assignToPartyId", s(&worker)),
            ("priority", n("7")),
            ("purposeEnumId", s("WepTask")),
            ("estimatedWorkTime", n("2")),
            ("description", s("")),
        ],
    );
    let task_id = need(&task, "workEffortId");
    call(
        "create#mantle.request.RequestWorkEffort",
        &[("workEffortId", s(&task_id)), ("requestId", s(&request_id))],
    );
    call(
        "mantle.work.TaskServices.update#Task",
        &[
            ("workEffortId", s(&task_id)),
            ("statusId", s("WeComplete")),
            ("resolutionEnumId", s("WerCompleted")),
        ],
    );
    call(
        "mantle.request.RequestServices.update#Request",
        &[("requestId", s(&request_id)), ("statusId", s("ReqCompleted"))],
    );

    // create Worker Time and Expense Invoice and record Payment
    let expense = call(
        "mantle.account.InvoiceServices.create#ProjectExpenseInvoice",
        &[
            ("workEffortId", s("TEST")),
            ("fromPartyId", s(&worker)),
            ("invoiceDate", s(&format!("{YEAR}-11-08"))),
        ],
    );
    let expense_id = need(&expense, "invoiceId");
    for (kind, description, date, amount) in [
        ("ItemExpTravAir", "United SFO-LAX", "11-02", "345.67"),
        ("ItemExpTravLodging", "Fleabag Inn 2 nights", "11-04", "123.45"),
    ] {
        call(
            "create#mantle.account.invoice.InvoiceItem",
            &[
                ("invoiceId", s(&expense_id)),
                ("itemTypeEnumId", s(kind)),
                ("description", s(description)),
                ("itemDate", s(&format!("{YEAR}-{date}"))),
                ("quantity", n("1")),
                ("amount", n(amount)),
            ],
        );
    }
    call(
        "mantle.account.InvoiceServices.create#ProjectInvoiceItems",
        &[
            ("invoiceId", s(&expense_id)),
            ("workerPartyId", s(&worker)),
            ("ratePurposeEnumId", s("RaprVendor")),
            ("workEffortId", s("TEST")),
            ("thruDate", s(&after_now_text())),
        ],
    );
    call(
        "update#mantle.account.invoice.Invoice",
        &[("invoiceId", s(&expense_id)), ("statusId", s("InvoiceReceived"))],
    );
    call(
        "mantle.account.PaymentServices.create#InvoicePayment",
        &[
            ("invoiceId", s(&expense_id)),
            ("statusId", s("PmntDelivered")),
            ("amount", s("1009.12")),
            ("paymentInstrumentEnumId", s("PiCompanyCheck")),
            ("effectiveDate", s(&format!("{YEAR}-11-10 12:00:00"))),
            ("paymentRefNum", s("1234")),
            ("comments", s("Delivered by Fedex")),
        ],
    );
    let expense_invoice = one(f, "mantle.account.invoice.Invoice", &[("invoiceId", &expense_id)]).unwrap();
    assert_eq!(number(expense_invoice.get("invoiceTotal")), 1009.12);
    let xml = format!(
        r#"<entity-facade-xml>
        <mantle.account.invoice.Invoice invoiceId="{expense_id}" invoiceTypeEnumId="InvoiceSales" fromPartyId="{worker}" toPartyId="{vendor_id}" statusId="InvoicePmtSent" currencyUomId="USD"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{expense_id}" itemTypeEnumId="ItemExpTravAir" quantity="1" amount="345.67" description="United SFO-LAX"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{expense_id}" itemTypeEnumId="ItemExpTravLodging" quantity="1" amount="123.45" description="Fleabag Inn 2 nights"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{expense_id}" itemTypeEnumId="ItemTimeEntry" quantity="2" amount="40"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{expense_id}" itemTypeEnumId="ItemTimeEntry" quantity="6" amount="40"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{expense_id}" itemTypeEnumId="ItemTimeEntry" quantity="4" amount="40"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{expense_id}" itemTypeEnumId="ItemTimeEntry" quantity="1.5" amount="40"/>
        <mantle.ledger.transaction.AcctgTrans acctgTransTypeEnumId="AttPurchaseInvoice" organizationPartyId="{vendor_id}" isPosted="Y" glFiscalTypeEnumId="GLFT_ACTUAL" amountUomId="USD" otherPartyId="{worker}" invoiceId="{expense_id}"/>
        <mantle.account.payment.Payment paymentTypeEnumId="PtInvoicePayment" fromPartyId="{vendor_id}" toPartyId="{worker}" paymentInstrumentEnumId="PiCompanyCheck" statusId="PmntDelivered" paymentRefNum="1234" amount="1009.12" amountUomId="USD"/>
        <mantle.account.payment.PaymentApplication invoiceId="{expense_id}" amountApplied="1009.12"/>
        </entity-facade-xml>"#
    );
    assert_clean(check(f, &xml), "create Worker Time and Expense Invoice and record Payment");
    let trans = list(
        f,
        "mantle.ledger.transaction.AcctgTrans",
        &[("invoiceId", &expense_id), ("acctgTransTypeEnumId", "AttPurchaseInvoice")],
    );
    assert_eq!(trans.len(), 1, "{trans:?}");
    let entries = list(
        f,
        "mantle.ledger.transaction.AcctgTransEntry",
        &[("acctgTransId", &text(&trans[0], "acctgTransId"))],
    );
    let sum = |flag: &str| -> f64 {
        entries
            .iter()
            .filter(|e| text(e, "debitCreditFlag") == flag)
            .map(|e| number(e.get("amount")))
            .sum()
    };
    assert_eq!(entries.len(), 7, "{entries:?}");
    assert!((sum("D") - 1009.12).abs() < 0.001 && (sum("C") - 1009.12).abs() < 0.001, "{entries:?}");

    // create Client Time and Expense Invoice and Finalize; record Payment
    let client_invoice = call(
        "mantle.account.InvoiceServices.create#ProjectInvoiceItems",
        &[
            ("ratePurposeEnumId", s("RaprClient")),
            ("workEffortId", s("TEST")),
            ("thruDate", s(&after_now_text())),
            ("invoiceDate", s(&format!("{YEAR}-11-08"))),
        ],
    );
    let client_invoice_id = need(&client_invoice, "invoiceId");
    call(
        "update#mantle.account.invoice.Invoice",
        &[
            ("invoiceId", s(&client_invoice_id)),
            ("statusId", s("InvoiceFinalized")),
        ],
    );
    call(
        "mantle.account.PaymentServices.create#InvoicePayment",
        &[
            ("invoiceId", s(&client_invoice_id)),
            ("statusId", s("PmntDelivered")),
            ("amount", n("1039.12")),
            ("paymentInstrumentEnumId", s("PiCompanyCheck")),
            ("effectiveDate", s(&format!("{YEAR}-11-12 12:00:00"))),
            ("paymentRefNum", s("54321")),
        ],
    );
    let paid = one(
        f,
        "mantle.account.invoice.Invoice",
        &[("invoiceId", &client_invoice_id)],
    )
    .unwrap();
    assert_eq!(text(&paid, "statusId"), "InvoicePmtRecvd");
    let xml = format!(
        r#"<entity-facade-xml>
        <mantle.account.invoice.Invoice invoiceId="{client_invoice_id}" invoiceTypeEnumId="InvoiceSales" fromPartyId="{vendor_id}" toPartyId="{client_id}" statusId="InvoicePmtRecvd" currencyUomId="USD"/>
        <mantle.account.invoice.Invoice invoiceId="{expense_id}" statusId="InvoiceBilledThrough"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{client_invoice_id}" itemTypeEnumId="ItemTimeEntry" quantity="2" amount="60"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{client_invoice_id}" itemTypeEnumId="ItemTimeEntry" quantity="6" amount="60"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{client_invoice_id}" itemTypeEnumId="ItemTimeEntry" quantity="4" amount="0"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{client_invoice_id}" itemTypeEnumId="ItemTimeEntry" quantity="1.5" amount="60"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{client_invoice_id}" itemTypeEnumId="ItemExpTravAir" quantity="1" amount="345.67" description="United SFO-LAX"/>
        <mantle.account.invoice.InvoiceItem invoiceId="{client_invoice_id}" itemTypeEnumId="ItemExpTravLodging" quantity="1" amount="123.45" description="Fleabag Inn 2 nights"/>
        <mantle.account.invoice.InvoiceItemAssoc invoiceId="{expense_id}" toInvoiceId="{client_invoice_id}" invoiceItemAssocTypeEnumId="IiatBillThrough" quantity="1" amount="345.67"/>
        <mantle.account.invoice.InvoiceItemAssoc invoiceId="{expense_id}" toInvoiceId="{client_invoice_id}" invoiceItemAssocTypeEnumId="IiatBillThrough" quantity="1" amount="123.45"/>
        <mantle.ledger.transaction.AcctgTrans acctgTransTypeEnumId="AttSalesInvoice" organizationPartyId="{vendor_id}" isPosted="Y" glFiscalTypeEnumId="GLFT_ACTUAL" amountUomId="USD" otherPartyId="{client_id}" invoiceId="{client_invoice_id}"/>
        <mantle.account.payment.Payment paymentTypeEnumId="PtInvoicePayment" fromPartyId="{client_id}" toPartyId="{vendor_id}" paymentInstrumentEnumId="PiCompanyCheck" statusId="PmntDelivered" paymentRefNum="54321" amount="1039.12" amountUomId="USD"/>
        <mantle.account.payment.PaymentApplication invoiceId="{client_invoice_id}" amountApplied="1039.12"/>
        </entity-facade-xml>"#
    );
    assert_clean(check(f, &xml), "client invoice and payment");
    let trans = list(
        f,
        "mantle.ledger.transaction.AcctgTrans",
        &[("invoiceId", &client_invoice_id), ("acctgTransTypeEnumId", "AttSalesInvoice")],
    );
    assert_eq!(trans.len(), 1, "{trans:?}");
    let entries = list(
        f,
        "mantle.ledger.transaction.AcctgTransEntry",
        &[("acctgTransId", &text(&trans[0], "acctgTransId"))],
    );
    assert_eq!(entries.len(), 6, "{entries:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// RestApiTests
// ---------------------------------------------------------------------------------------------------------------

fn flow_rest_api(f: &Flow) {
    let _ = f;
    login("john.doe");
    // calculate GL Account Org Summaries for Accounting Tests
    call("mantle.ledger.LedgerServices.recalculate#GlAccountOrgSummaries", &[]);
    // The remaining Spock cases render `s1/mantle/...` REST screens through ScreenTest and check the output text
    // (orders/productQuantity, shippingBilling, place, approve, payments, reservations, shipments, billings,
    // issuances). They need the REST screen renderer driven from a test, which this harness does not have yet.
    panic!("REST screen cases are not ported: no ScreenTest equivalent in this harness");
}

// ---------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------

macro_rules! flow_test {
    ($name:ident, $flow:expr, $function:ident, $reason:expr) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            with_flow($flow, $function);
        }
    };
}

flow_test!(
    order_to_cash_basic_flow,
    "OrderToCashBasicFlow",
    flow_order_to_cash_basic,
    "OrderToCashBasicFlow: first failing service mantle.product.PriceServices.get#ProductPrice (still Groovy)"
);
flow_test!(
    order_procure_to_pay_basic_flow,
    "OrderProcureToPayBasicFlow",
    flow_order_procure_to_pay,
    "OrderProcureToPayBasicFlow: first failing service mantle.product.PriceServices.get#ProductPrice (still Groovy)"
);
flow_test!(
    return_to_response_basic_flow,
    "ReturnToResponseBasicFlow",
    flow_return_to_response,
    "ReturnToResponseBasicFlow: runs OrderToCashBasicFlow's order first, so fails at the same service"
);
#[test]
fn work_plan_to_cash_basic_flow() {
    with_flow("WorkPlanToCashBasicFlow", flow_work_plan_to_cash);
}
#[test]
fn accounting_activities() {
    with_flow("AccountingActivities", flow_accounting_activities);
}
flow_test!(
    order_to_cash_time,
    "OrderToCashTime",
    flow_order_to_cash_time,
    "OrderToCashTime: first failing service mantle.order.OrderServices.add#OrderProductQuantity"
);
flow_test!(
    asset_reservation_multiple_threads,
    "AssetReservationMultipleThreads",
    flow_asset_reservation_multiple_threads,
    "AssetReservationMultipleThreads: first failing service mantle.order.OrderServices.add#OrderProductQuantity"
);
flow_test!(
    order_tenant_access,
    "OrderTenantAccess",
    flow_order_tenant_access,
    "OrderTenantAccess: not run by the Spock suite either (placeholder for instance access); fails at add#OrderProductQuantity"
);
flow_test!(
    rest_api_tests,
    "RestApiTests",
    flow_rest_api,
    "RestApiTests: first failing service mantle.ledger.LedgerServices.recalculate#GlAccountOrgSummaries (Groovy); the REST screen cases need a ScreenTest equivalent and are not ported"
);

/// MantleUslSuite: the flows in the suite's order, on one database.
#[test]
#[ignore = "MantleUslSuite: runs the flows in suite order; fails at AccountingActivities, get#OrganizationFiscalTimePeriods"]
fn mantle_usl_suite() {
    with_flow("MantleUslSuite", |f| {
        flow_accounting_activities(f);
        flow_asset_reservation_multiple_threads(f);
        flow_order_procure_to_pay(f);
        flow_order_to_cash_basic(f);
        flow_order_to_cash_time(f);
        flow_return_to_response(f);
        flow_work_plan_to_cash(f);
        flow_rest_api(f);
    });
}
