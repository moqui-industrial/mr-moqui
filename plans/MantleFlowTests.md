# Mantle flow tests (Phase 0b)

Rust integration tests that drive the same service calls and entity assertions as the ten Spock specs in `runtime/component/mantle-usl/src/test/groovy`. The Groovy files stay as the oracle and are not edited. The point is a safety net for the JavaScript conversion of mantle-usl: a flow that passes here still passes after the services it calls are ported.

Files:

- `crates/moqui-host/tests/mantle_flows.rs`: one test per flow, the service table (`FLOW_SERVICES`), and `service_readiness`.
- `crates/moqui-host/tests/mantle_support/mod.rs`: the shared fixture and helpers.

## Running

```sh
cargo test -p moqui-host --test mantle_flows -- --test-threads=1            # the flows that pass, and service_readiness
cargo test -p moqui-host --test mantle_flows -- --ignored                 # every flow, in parallel threads, serialized by a lock
cargo test -p moqui-host --test mantle_flows order_to_cash -- --ignored --nocapture    # one flow, with its service calls printed
cargo test -p moqui-host --test mantle_flows service_readiness -- --nocapture          # which services register
```

Without `runtime/component/mantle-udm` and `mantle-usl` every test returns at once. A flow's first failure stops it; the panic starts with `SERVICE <name>:` and, for a service that did not register, says why (the Groovy expression, or the missing service).

When a flow starts passing, delete its `#[ignore = "..."]` line so it runs in the default `cargo test`.

## The fixture

`with_flow` starts one scratch runtime and embedded Postgres per test binary (under `/tmp`, short path for the socket), runs `moqui load types=all` (seed, seed-initial, install, demo; about 20 seconds), boots the factory, and removes everything at process exit (`atexit`). Each flow then runs with an entered context and authorization disabled, as the Spock `setup()` does. The flows share the database, as the Spock specs share a JVM, and run one at a time. `ReturnToResponseBasicFlow` needs OrderToCashBasicFlow's order; it runs that flow's first four features itself if they have not run.

Helpers: `call` (service sync; a thrown error or a message-facade error panics naming the service), `login`/`logout`, `one`/`list` (entity finds), `check` (the port of `makeDataLoader().xmlText(..).check()`).

Differences from the Spock files:

- No `tempSetSequencedIdPrimary` and no `setEffectiveTime`. Generated ids (orders, shipments, invoices, payments) are read from service results; ids that Spock pins (`55500`, `55400`) are looked up by their natural keys. Demo-data ids such as asset `55400` and `DEMO_3_1A` are still asserted. Dates are left out of the checks.
- `check` matches rows by the `...Id` attributes and compares the other attributes (numbers by value, the rest as text). It skips nested relationship elements (`<items>`, `<receipts>`); those are asserted with explicit finds in the port.
- The Spock files assume an empty ledger; the AccountingActivities port asserts deltas of `GlAccountOrgTimePeriod` postings.
- Timing: Spock logs elapsed time in OrderToCashTime and asserts nothing; the port does the same.

## Status

No flow passes yet. Every flow stops at its first Groovy service; steps after that have not run, so assertions after the first failure are untested ports. `service_readiness` passes.

| Flow | Status | First failing service | Why |
|---|---|---|---|
| OrderToCashBasicFlow | passes (runs by default) | | runs after AssetReservationMultipleThreads and OrderProcureToPayBasicFlow, as the suite does (the flow depends on their orders and stock) |
| OrderProcureToPayBasicFlow | passes (runs by default) | | runs AssetReservationMultipleThreads first |
| ReturnToResponseBasicFlow | passes (runs by default) | | runs the first features of OrderToCashBasicFlow (after its prerequisites) for the order it returns from |
| WorkPlanToCashBasicFlow | passes (runs by default) | | the worker and client invoices, GL transactions, payments and bill-through are asserted |
| AccountingActivities | ignored, fails | `mantle.ledger.LedgerServices.get#OrganizationFiscalTimePeriods` | registers, but `new java.sql.Date(filterDate.time)` fails at run time (`cannot read property 'sql' of null`); `post#AcctgTrans` is Groovy |
| OrderToCashTime | passes (runs by default) | | |
| AssetReservationMultipleThreads | passes (runs by default) | | five threads each with their own context |
| OrderTenantAccess | ignored | `mantle.order.OrderServices.add#OrderProductQuantity` | `Product DEMO_TNT not found`: multi-tenant support is deprecated and the product is commented out in ZcaProductDemoData.xml. The Spock suite does not run this spec either |
| RestApiTests | ignored, fails | `mantle.ledger.LedgerServices.recalculate#GlAccountOrgSummaries` | `new HashSet(timePeriodList*.partyId)` and other Groovy. The `s1/mantle/...` REST screen cases are not ported: the harness has no ScreenTest equivalent |
| MantleUslSuite | ignored, fails | AccountingActivities, `get#OrganizationFiscalTimePeriods` | runs the flows in the suite's order (AccountingActivities, AssetReservationMultipleThreads, OrderProcureToPay, OrderToCash, OrderToCashTime, ReturnToResponse, WorkPlanToCash, RestApi) |

Port coverage: OrderToCashBasicFlow (all 18 features, the five posting and invoice validations folded into four steps), AccountingActivities (all 6), OrderToCashTime, AssetReservationMultipleThreads, OrderTenantAccess, ReturnToResponseBasicFlow (the return, receive, replacement, refund, credit and purchase-with-credit features, with row-level asserts instead of the full XML checks), OrderProcureToPayBasicFlow (the purchase order through shrinkage and found; the two asset sales share one loop; per-record GL entry checks are reduced to the key totals), WorkPlanToCashBasicFlow (all features; worker/client invoice amounts are asserted at the invoice level). The detailed `AcctgTransEntry` listings of the Groovy files are asserted only for AccountingActivities' expense invoice and the sales order's issuance and invoice postings.

## Services readiness

`service_readiness` registers every service a flow calls by name and reports which register. Registered today of those called (a service that fails to register is Groovy or has an unknown `implements`):

| Flow | Registered / called |
|---|---|
| OrderToCashBasicFlow | 14 / 25 |
| OrderProcureToPayBasicFlow | 14 / 23 |
| ReturnToResponseBasicFlow | 7 / 11 |
| WorkPlanToCashBasicFlow | passes (runs by default) | | the worker and client invoices, GL transactions, payments and bill-through are asserted |
| AccountingActivities | 6 / 7 |
| OrderToCashTime | 1 / 5 |
| AssetReservationMultipleThreads | 1 / 4 |
| OrderTenantAccess | 0 / 3 |
| RestApiTests | 0 / 1 |

Services the flows reach only through SECA and EECA rules (invoice on ship, payment application, GL posting) are not in this count; they surface as the flow fails.

## Services each flow calls

See the `FLOW_SERVICES` table in `mantle_flows.rs` for the full lists. The services that block the most flows: `OrderServices.add#OrderProductQuantity`, `set#OrderBillingShippingInfo`, `place#Order` (all 6 order flows), `PriceServices.get#ProductPrice` (3), `ShipmentServices.ship#OrderPart` (3), `PaymentServices.create#InvoicePayment` (2).
