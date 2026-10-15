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

## Known differences from the Groovy checks

- `totalFieldsChecked` counting is dropped; `checkData` returns only errors.
- `passwordHashType`: this host hashes with argon2, the Groovy expects `SHA-256`.
- A date-only `fromDate` parses as UTC midnight, the Groovy expected US Central midnight.
- REST JSON is written `"key": value`, Jackson wrote `"key" : value`.
- `calculate#FixedAssetDepreciation` and the "sell Depreciated Asset Gain" check depend on the process time zone being behind UTC, as the Groovy does.
