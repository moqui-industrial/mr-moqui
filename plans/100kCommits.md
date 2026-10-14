# Durable invoice: less client work, then faster durable storage

## What changes
`experiments/mantle-execution/invoice-fused.sql` is the optimized statement, not a new framework.
Prepare its contents once per connection; bind invoice/worker ID and unique request sequence.
It replaces BEGIN → three SELECTs → client calculation → UPDATE → receipt INSERT → COMMIT
with one synchronous autocommit: aggregate items/payments, update header, insert its receipt.
This removes six serial commands, row transfer and client decoding/arithmetic; update/receipt
remain atomic. Count success only after durable execution returns with one inserted receipt.
No runner, generated fixtures, dependencies or unrelated tuning are included.
Extracted SQL passed 48 exact-decimal cases on disposable PostgreSQL 18; no new throughput claim.
## Before and after
`origin/initial` (`4e02783`) has no executable invoice benchmark or measured full-framework rate.
Its plan cites historical ~26k/s faithful and 35.8k/s sustained fused reduced workloads;
the original 41-worker, 100-line table measured 2,814–2,937/s without p99. These are not matched.
Below are matched results from experiment `d72c79f`, not new runs of this extracted SQL file:

| Metric | Before: host helper | After: fused SQL | Change |
|---|---:|---:|---:|
| 1,024 workers: durable commits/s | 15,567 | 30,397 | +95.3% |
| 1,024 workers: p99 | 498.536 ms | 296.228 ms | -40.6% |
| Application CPU/commit | 202.62 us | 30.33 us | -85.0% |
| Application + PostgreSQL CPU/commit | 633.32 us | 291.66 us | -53.9% |
| 64 workers: durable commits/s | 2,301 | 2,264 | -1.6% |
| 64 workers: p99 | 52.236 ms | 53.270 ms | +2.0% |

Medians of three alternating 60-second pairs at 1,024 workers; five pairs at 64.
All three high-concurrency pairs improved throughput/p99; lower concurrency did not.
Six high-concurrency runs validated 8,374,714 receipts and post-drain recovery.
CPU includes setup/validation; desktop load was uncontrolled. Not a production pool recommendation.
30.4k/s is only 30.4% of 100k/s; this bypasses full service semantics and is not target acceptance.
Evidence: local `backup/durable-invoice-d72c79f`, `experiments/mantle-execution/` results and reports.

## Primary bottleneck: waiting for the SSD to persist WAL
Initial diagnostics measured **9.42 ms mean PostgreSQL WAL sync** on the Samsung 960 PRO,
with encrypted Btrfs. 97.8% of active-backend samples were at COMMIT; 94.9% waited on
WALWrite and 2.6% on WalSync. These are sample shares, not request-latency percentages.
WAL traffic was only ~0.27 MB/s: persistence latency/coordination, not bandwidth exhaustion.
Independent file-sync tracing measured 6.740 ms average, with 6.158 ms (~91%) between
NVMe flush submission and the subsequent forced-durable (FUA) write completion.
The durability-command path is slow on this machine; this does not prove defective firmware
or mean every request spends exactly 9 ms inside the SSD. Group commit shares persistence waits.
SQL fusion reduces CPU/exchanges but **does not remove or repair the storage bottleneck**.

## Future work to improve it
1. Retest on an otherwise idle machine; record WAL normal/initialization sync p50/p99,
   group sizes, queue depth, checkpoints and application/database CPU alongside request p99.
2. Compare genuinely lower-latency, power-loss-protected WAL storage under durable flush load,
   including concurrent checkpoint traffic. Separate WAL physically if contention is measured;
   a partition or additional database on this same SSD does not create independent capacity.
3. Isolate filesystem/encryption effects on disposable storage before blaming/replacing hardware.
   NOCOW removes Btrfs data checksums; do not change the production filesystem blindly.
4. Keep PostgreSQL native synchronous group commit and bounded admission. At 100k/s a 9.42 ms
   group interval would require ~942 commits/group; 0.1–0.5 ms would require ~10–50.
   These are illustrative budgets, not measured group sizes or guaranteed storage throughput.
5. Rerun >=15-minute paired tests with normal checkpoints/autovacuum and the real endpoint,
   authorization/validation/ECA/audit/feed work, admission-inclusive latency and an absolute p99 SLO.
Never disable fsync/full_page_writes, lie about flush completion or acknowledge before durability.
## Test contract (for rebuilding the runner)
Use release Rust, PostgreSQL 18.6, persistent disk, cached connections/statements, 128MB shared
buffers, fdatasync, synchronous_commit=on, commit_delay=0 and wal_init_zero=on; fresh DB per run.
Create bench_invoice(id PK,total,applied,unpaid), bench_invoice_item(invoice_id,item_seq,quantity,
amount; composite PK), bench_payment(invoice_id indexed,amount), bench_receipt(worker,request_seq,
total,applied,unpaid; composite PK); all decimal columns numeric(26,6). One invoice per worker.
Seed 100 items cycling (quantity,amount): (null,null),(0,0),(0,2),(2,12.345),(0,12.345),
(2,12.345),(1,0.125),(1,-0.125),(1,1.005),(1,-1.005),(0.25,19.999),(-2,12.345),
(2,12.345),(null,12.345),(2,null),(1000000,999999.999999); payments 12.345 and null.
Expected total/applied/unpaid: 6000000000419.07 / 12.345 / 6000000000406.725.
Check 1/10/100 lines at all offsets against independent exact-decimal arithmetic before timing.
Preserve per-line half-away-from-zero rounding, null defaults, zero quantity and payment precision.
Seed/commit/checkpoint before timing. Synchronize workers; FIFO capacity equals worker count.
Measure closed-loop requests for 60 seconds including admission/commit latency and final drain;
rate=durable completions/elapsed time, p99=sorted duration at floor((N-1)*0.99), 100% writes.
Validate every header/receipt, submitted=completed=receipt count and contiguous sequences from zero.
After drain, hash receipts; SIGKILL disposable DB, restart, compare count/hash (not power-loss proof).
Use five alternating pairs at 64 workers and three at 1,024; accept higher rate AND no-worse p99
in every pair. Inject duplicate/failing receipts: no success or partially committed header update.
Fusion changes statement snapshots; only equivalent here for immutable items/payments, independent
invoices and no hooks/competing writers. Preserve real-service semantics before integrating.

## Why not the other options?
Deferred acknowledgment queues reached 26–29k/s but p99 367–401 ms versus synchronous 27–35 ms
at 64 workers. Smaller pools reduced throughput/worsened p99; native helpers leave exchanges intact.
wal_init_zero=off regressed rate 17% and p99 5%. More connections, same-SSD shards and added commit
delay are not automatic wins. Faster arithmetic alone cannot fix millisecond durability waits.
