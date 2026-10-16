# Mantle USL flow tests

The ten Spock specs of mantle-usl (`runtime/component/mr-mantle-usl/src/test/groovy`, kept as the oracle) are ported to JavaScript specs in `runtime/component/mr-mantle-usl/test/`, run by `moqui test` (design: [TestHarness](TestHarness.md)). The earlier Rust port (`mantle_flows.rs`, `mantle_support`) is deleted.

```sh
moqui test mantle-usl                                  # every spec, fresh database
moqui test mantle-usl --db snapshot                    # load once, restore later
moqui test mantle-usl --spec OrderToCashBasicFlow      # one spec and its dependsOn closure
```

The component name is `mantle-usl` (from `component.xml`), not the directory name.

## Status (2026-10-15)

84 features pass in 8 specs, none ignored (about 130s).

| Spec | Features |
|---|---|
| AccountingActivities | 6 |
| AssetReservationMultipleThreads | 1 |
| OrderProcureToPayBasicFlow | 23 |
| OrderToCashBasicFlow | 42 |
| OrderToCashTime | 1 |
| ReturnToResponseBasicFlow | 12 |
| WorkPlanToCashBasicFlow | 11 |
| RestApiTests | 1 summaries + 11 `where` rows |

`OrderTenantAccess` is not ported (not in the Spock suite; multi-tenant is deprecated).

## Specs without a Spock original (2026-10-16)

Written to run services that no flow reaches, and run after the flows. Each was written against the service text, not an oracle, so a failing check is as likely a wrong expectation as a bug; the notes in each file say where a service is an upstream stub. `moqui test mantle-usl --coverage` lists the services no spec called, per file; `--coverage-report file` writes the call counts.

| Spec | Services |
|---|---|
| HumanResServices | Employment, EmploymentSalary, RateAmount, payroll time periods, positions |
| PayrollServices | process#TimePeriodPay, employment pay, invoices, payments, reopen |
| ContactAndFacilityServices | party and facility contact info, FacilityServices |
| PartyServices | find, convert, roles, classifications, settings, user accounts, badge scans |
| BankFileServices | OFX and BAI 2 import, match, reconcile, unreconcile |
| BankOutputFiles | NACHA, Positive Pay (six formats) |
| FinancialReports | income statement, balance sheet, cash flow, ratios, ledger info, period series |
| ProductCatalogServices | features, variants, categories, content, clone, store slugs and options |
| BudgetServices | budgets, sub-period items, details, expense reports |
| TaxStatementServices | 1099-MISC statements, payer and recipient tax info |
| InvoicePaymentMisc | invoice and payment edits, credit memo, reverse, cancel, void, refund |
| WorkProjectServices | work efforts, projects, milestones, tasks, comments, time entries, events |
| LedgerMaintenance | GL accounts, transactions, journals, period close and reopen, reposting |

Specs that create their own organization (`init#PartyAccountingConfiguration` with `sourcePartyId: "DefaultSettings"`) get a full set of fiscal periods and accounts. A spec that freezes time with `ec.user.setEffectiveTime` cannot give one party the same role twice in the same millisecond (primary key).

## Known differences from the Groovy checks

- `totalFieldsChecked` counting is dropped; `checkData` returns only errors.
- `passwordHashType`: this host hashes with argon2, the Groovy expects `SHA-256`.
- A date-only `fromDate` parses as UTC midnight, the Groovy expected US Central midnight.
- REST JSON is written `"key": value`, Jackson wrote `"key" : value`.
- `calculate#FixedAssetDepreciation` and the "sell Depreciated Asset Gain" check depend on the process time zone being behind UTC, as the Groovy does.
